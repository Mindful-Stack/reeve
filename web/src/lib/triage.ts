// web/src/lib/triage.ts: the triage page's pure rules. The queue's order, an item's draft against the base it was loaded
// with, the Place and Undo bodies (only changed fields, kb/ADR-0018 R4: placing sets the epic, nothing else marks an
// item triaged), the merge after a reload or a conflict, duplicate drops, and the drop arm.
import type { EditItem, SimilarStory, Story } from '#lib/api/types.ts';
import type { Kind } from '#lib/board.ts';
import { changed, mergeDraft, type Edit, type Same } from '#lib/forms.ts';

type Criterion = Story['criteria'][number];

const keyNumber = (key: string) => {
	const n = Number(key.slice(key.lastIndexOf('-') + 1));
	return Number.isFinite(n) ? n : Number.MAX_SAFE_INTEGER;
};

/** The queue oldest first: keys go up with creation, so the number after the dash orders them (kb/ADR-0021 R1: the
 *  server filtered; this only sorts what it sent, or an item Undo put back). */
export function byKey(stories: Story[]): Story[] {
	return [...stories].sort((a, b) => keyNumber(a.key) - keyNumber(b.key) || a.key.localeCompare(b.key));
}

/** The form's fields as text: epic ('' for none), kind, title, criteria one per line. */
export type Fields = { epic: string; kind: Kind; title: string; criteria: string };
export type Base = { version: number; fields: Fields; criteria: Criterion[] };
/** `duplicate`: the similar item marked as this one's duplicate, or null. It lives and dies with the draft. */
export type Draft = { base: Base; fields: Fields; duplicate: string | null };

const lines = (t: string) => t.split('\n').map((l) => l.trim()).filter((l) => l !== '');
const exact = (a: string, b: string) => a === b;
const trimmed = (a: string, b: string) => a.trim() === b.trim();
const sameLines = (a: string, b: string) => JSON.stringify(lines(a)) === JSON.stringify(lines(b));
const TRIAGE_SAME: Same<Fields> = { epic: exact, kind: exact, title: trimmed, criteria: sameLines };
export const FIELD_LABEL: Record<keyof Fields, string> = { epic: 'Epic', kind: 'Kind', title: 'Title', criteria: 'Criteria' };

function fieldsOf(s: Story): Fields {
	return { epic: s.epic ?? '', kind: s.kind, title: s.title, criteria: s.criteria.map((c) => c.text).join('\n') };
}

function baseOf(s: Story): Base {
	return { version: s.version, fields: fieldsOf(s), criteria: s.criteria };
}

export function draftOf(s: Story): Draft {
	const base = baseOf(s);
	return { base, fields: { ...base.fields }, duplicate: null };
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
	if (!d.fields.title.trim()) return { kind: 'invalid', error: 'Title must not be blank.' };
	const body: EditItem = { version: d.base.version };
	for (const k of changed(TRIAGE_SAME, d.base.fields, d.fields)) {
		if (k === 'epic') body.epic = d.fields.epic;
		else if (k === 'kind') body.kind = d.fields.kind;
		else if (k === 'title') body.title = d.fields.title.trim();
		else body.criteria = criteriaFromText(d.fields.criteria, d.base.criteria);
	}
	return { kind: 'send', body };
}

/** After a 409, or on a reload: the base becomes the server's; the user's changed fields are kept and every other takes
 *  the server's value (`mergeDraft`); `clashes` are the fields both sides changed, with the server's value now. */
export function rebaseAfterConflict(d: Draft, s: Story): { draft: Draft; clashes: { field: keyof Fields; value: string }[] } {
	const server = fieldsOf(s);
	const clashes = changed(TRIAGE_SAME, d.base.fields, d.fields)
		.filter((k) => !TRIAGE_SAME[k](d.base.fields[k], server[k]))
		.map((k) => ({ field: k, value: server[k] }));
	return { draft: { base: baseOf(s), fields: mergeDraft(TRIAGE_SAME, d.base.fields, d.fields, server), duplicate: d.duplicate }, clashes };
}

/** On a reload: the base follows the server unless a field the user changed has changed there too; then the draft is kept
 *  as it is, and Place conflicts rather than overwrite. One rule with the conflict merge: they differ only on a clash.
 *  An unchanged item leaves the draft alone: the merge would put the server's text over a field that differs only by
 *  spacing (a trailing space or a new empty line, the one being typed). */
export function followDraft(d: Draft, s: Story): Draft {
	if (s.version === d.base.version) return d;
	const r = rebaseAfterConflict(d, s);
	return r.clashes.length ? d : r.draft;
}

/** The PATCH that undoes a Place: every field the Place sent, back to its base value, at the version the Place returned. */
export function undoEdit(placed: Story, before: Draft, sent: EditItem): EditItem {
	const body: EditItem = { version: placed.version };
	if ('epic' in sent) body.epic = before.base.fields.epic || null;
	if ('kind' in sent) body.kind = before.base.fields.kind;
	if ('title' in sent) body.title = before.base.fields.title;
	if ('criteria' in sent) body.criteria = before.base.criteria;
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

/** The server's similar list includes the item itself: drop it and keep five. */
export function withoutSelf(hits: SimilarStory[], key: string): SimilarStory[] {
	return hits.filter((h) => h.key !== key).slice(0, 5);
}

/** The text `GET /stories/similar` accepts (8 to 200 characters of the trimmed title, counted in code points as the
 *  server and the Report dialog count), or null when the title is outside that. */
export function similarQuery(title: string): string | null {
	const t = title.trim();
	const n = [...t].length;
	return n >= 8 && n <= 200 ? t : null;
}

/** Where an item that left the queue went. */
export function whereNow(s: Story): string {
	if (s.delivery === 'done') return `${s.key} is done (${s.resolution ?? 'completed'}).`;
	return s.epic ? `Placed elsewhere: ${s.key} is under ${s.epic}.` : `${s.key} changed.`;
}

export type Arm = { key: string; version: number; at: number };
export const ARM_MS = 3000;

/** A second `d` drops only the item and version the first armed, within ARM_MS. Marking a duplicate disarms, so the arm
 *  need not name one. */
export function armedFor(a: Arm | null, key: string, version: number, now: number): boolean {
	return !!a && a.key === key && a.version === version && now - a.at < ARM_MS;
}
