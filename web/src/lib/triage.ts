// web/src/lib/triage.ts: the triage page's pure rules. The queue's order, an item's draft against the base it was loaded
// with, the Place and Undo bodies (only changed fields, kb/ADR-0018 R4: placing sets the epic, nothing else marks an
// item triaged), the merge after a reload or a conflict, and duplicate drops.
import type { EditItem, Story } from '#lib/api/types.ts';
import type { Kind } from '#lib/board.ts';
import { BLANK_TITLE, changed, exact, trimmed, type Edit, type Same } from '#lib/forms.ts';

type Criterion = Story['criteria'][number];

/** Keys are `PREFIX-<n>`, one prefix per instance. */
const keyNumber = (key: string) => Number(key.slice(key.lastIndexOf('-') + 1));

/** The queue oldest first: keys go up with creation, so the number after the dash orders them (kb/ADR-0021 R1: the
 *  server filtered; this only sorts what it sent, or an item Undo put back). */
export function byKey(stories: Story[]): Story[] {
	return [...stories].sort((a, b) => keyNumber(a.key) - keyNumber(b.key));
}

/** The form's fields as text: epic ('' for none), kind, title, criteria one per line. */
type Fields = { epic: string; kind: Kind; title: string; criteria: string };
type Base = { version: number; fields: Fields; criteria: Criterion[] };
/** `duplicateOf`: the similar item this one is marked as a duplicate of, or null. It lives and dies with the draft. */
export type Draft = { base: Base; fields: Fields; duplicateOf: string | null };

const lines = (t: string) => t.split('\n').map((l) => l.trim()).filter((l) => l !== '');
const sameLines = (a: string, b: string) => JSON.stringify(lines(a)) === JSON.stringify(lines(b));
const TRIAGE_SAME: Same<Fields> = { epic: exact, kind: exact, title: trimmed, criteria: sameLines };
export const FIELD_LABEL: Record<keyof Fields, string> = { epic: 'Epic', kind: 'Kind', title: 'Title', criteria: 'Criteria' };
/** Every field, from the record that fails the build when a field is added: the lists below follow it. */
const FIELDS = Object.keys(FIELD_LABEL) as (keyof Fields)[];

function fieldsOf(s: Story): Fields {
	return { epic: s.epic ?? '', kind: s.kind, title: s.title, criteria: s.criteria.map((c) => c.text).join('\n') };
}

function baseOf(s: Story): Base {
	return { version: s.version, fields: fieldsOf(s), criteria: s.criteria };
}

export function draftOf(s: Story): Draft {
	const base = baseOf(s);
	return { base, fields: { ...base.fields }, duplicateOf: null };
}

/** One criterion per non-blank line, trimmed; a line equal to the base's text at the same position keeps its done flag
 *  (the rule the CLI's `--criterion` follows), any other starts not done. */
export function criteriaFromText(text: string, base: Criterion[]): Criterion[] {
	return lines(text).map((t, i) => ({ text: t, done: base[i]?.text === t ? base[i].done : false }));
}

/** The PATCH for Place: the base version and only the fields that changed. An epic is required, so there is always
 *  something to send. */
export function placeEdit(d: Draft): Exclude<Edit<EditItem>, { kind: 'unchanged' }> {
	if (!d.fields.epic) return { kind: 'invalid', error: 'Pick an epic first.' };
	if (!d.fields.title.trim()) return { kind: 'invalid', error: BLANK_TITLE };
	const body: EditItem = { version: d.base.version };
	for (const k of changed(TRIAGE_SAME, d.base.fields, d.fields)) {
		if (k === 'epic') body.epic = d.fields.epic;
		else if (k === 'kind') body.kind = d.fields.kind;
		else if (k === 'title') body.title = d.fields.title.trim();
		else if (k === 'criteria') body.criteria = criteriaFromText(d.fields.criteria, d.base.criteria);
		else k satisfies never; // a new field must say here how Place sends it
	}
	return { kind: 'send', body };
}

const assign = <K extends keyof Fields>(f: Fields, k: K, v: Fields[K]): void => {
	f[k] = v;
};

/** After a 409, or on a reload: the base becomes the server's. One rule per field: it takes the server's value unless the
 *  user changed it or already means what the server has (spacing, a line being typed). `clashes` are the fields both
 *  sides changed, with the server's value now. */
export function rebaseAfterConflict(d: Draft, s: Story): { draft: Draft; clashes: { field: keyof Fields; value: string }[] } {
	const server = fieldsOf(s);
	const mine = new Set(changed(TRIAGE_SAME, d.base.fields, d.fields));
	const fields = { ...d.fields };
	for (const k of FIELDS) if (!mine.has(k) && !TRIAGE_SAME[k](d.fields[k], server[k])) assign(fields, k, server[k]);
	const clashes = FIELDS.filter((k) => mine.has(k) && !TRIAGE_SAME[k](d.base.fields[k], server[k]) && !TRIAGE_SAME[k](d.fields[k], server[k])).map(
		(k) => ({ field: k, value: server[k] })
	);
	return { draft: { base: baseOf(s), fields, duplicateOf: d.duplicateOf }, clashes };
}

/** On a reload: the base follows the server unless a field the user changed has changed there too; then the draft is kept
 *  as it is, and Place conflicts rather than overwrite. One rule with the conflict merge: they differ only on a clash.
 *  An unchanged item leaves the draft alone: the merge would put the server's text over a field that differs only by
 *  spacing (a trailing space or a new empty line, the one being typed). */
export function followDraft(d: Draft, s: Story): Draft {
	if (s.version <= d.base.version) return d;
	const r = rebaseAfterConflict(d, s);
	return r.clashes.length ? d : r.draft;
}

/** The PATCH that undoes a Place: every field the Place sent, back to its base value, at the version the Place returned. */
export function undoEdit(placed: Story, before: Draft, sent: EditItem): EditItem {
	const body: EditItem = { version: placed.version };
	for (const k of FIELDS) {
		if (!(k in sent)) continue;
		if (k === 'epic') body.epic = before.base.fields.epic || null;
		else if (k === 'kind') body.kind = before.base.fields.kind;
		else if (k === 'title') body.title = before.base.fields.title;
		else if (k === 'criteria') body.criteria = before.base.criteria;
		else k satisfies never; // in step with placeEdit
	}
	return body;
}

/** The item after `key` in `order`, else the one before, else none. */
export function nextKey(order: string[], key: string): string | null {
	const i = order.indexOf(key);
	return order[i + 1] ?? order[i - 1] ?? null;
}

export const duplicateText = (dup: string) => `Duplicate of ${dup}.`;
/** One comment per item and duplicate: a retry, or a drop again after Undo, never posts a second one. */
export const duplicateCommentKey = (key: string, dup: string) => `triage-dup:${key}:${dup}`;

/** Where an item went once it is no longer untriaged (it has an epic, or it is done: kb/ADR-0018 R4), else null. The one
 *  place the client re-derives that: `GET /stories/{key}` carries no readiness, and the list that did say so is stale. */
export function whereNow(s: Story): string | null {
	if (s.delivery === 'done') return `${s.key} is done${s.resolution ? ` (${s.resolution})` : ''}.`;
	return s.epic ? `Placed elsewhere: ${s.key} is under ${s.epic}.` : null;
}
