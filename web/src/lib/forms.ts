// Pure helpers for the create and edit forms: request bodies (never a hand-written DTO: the types are the generated
// ones), the text a draft's idempotency key follows, target-month validation, per-field diffs and the per-field merge
// after a conflict.
import type { CreateEpic, CreateInitiative, EditDecision, EditItem, Epic, Initiative, RequiredDecision, Reversibility } from '#lib/api/types.ts';
import { sameEmail } from '#lib/decisions.ts';

/** `YYYY-MM` with a month 01–12: the shape `month()` in #lib/format.ts renders. */
export function validMonth(s: string): boolean {
	return /^\d{4}-(0[1-9]|1[0-2])$/.test(s);
}

/** A create body without its idempotency key, and `text`, the serialised content its draft key follows
 *  (#lib/idempotency.ts): equal for equal content sent, different when any sent field differs. */
export type Prepared<T> = { ok: true; body: T; text: string } | { ok: false; error: string };
/** An edit: the fields that changed, nothing to send, or why it cannot be sent. */
export type Edit<T> = { kind: 'send'; body: T } | { kind: 'unchanged' } | { kind: 'invalid'; error: string };

export const BLANK_TITLE = 'Title must not be blank.';
const BAD_MONTH = 'Target month must be YYYY-MM.';

export type InitiativeDraft = { title: string; horizon: string; target_month: string; brief: string };

export function initiativeCreate(d: InitiativeDraft, buckets: readonly string[]): Prepared<Omit<CreateInitiative, 'idempotency_key'>> {
	const title = d.title.trim();
	if (!title) return { ok: false, error: BLANK_TITLE };
	if (!buckets.includes(d.horizon)) return { ok: false, error: 'Choose a horizon.' };
	const month = d.target_month.trim();
	if (month && !validMonth(month)) return { ok: false, error: BAD_MONTH };
	const body = { title, horizon: d.horizon, target_month: month || null, brief: d.brief.trim() ? d.brief : null, owner: null };
	return { ok: true, body, text: JSON.stringify(body) };
}

/** `initiative` is a key, or '' for none. */
export type EpicDraft = { title: string; brief: string; initiative: string };

export function epicCreate(d: EpicDraft): Prepared<Omit<CreateEpic, 'idempotency_key'>> {
	const title = d.title.trim();
	if (!title) return { ok: false, error: BLANK_TITLE };
	const body = { title, brief: d.brief.trim() ? d.brief : null, initiative: d.initiative || null };
	return { ok: true, body, text: JSON.stringify(body) };
}

/** A stable text of a form's fields, for `EditorBase` (#lib/editorBase.ts): equal exactly when every field is. */
export function fieldsText(d: Record<string, string>): string {
	return JSON.stringify(Object.keys(d).sort().map((k) => [k, d[k]]));
}

/** Per field, when two values of a form mean the same thing. The one rule both the edit body (what is sent) and the
 *  merge after a conflict (what is kept) use, so the two can never disagree about whether the user changed a field. */
export type Same<T> = { [K in keyof T]: (a: string, b: string) => boolean };

export const trimmed = (a: string, b: string) => a.trim() === b.trim();
export const exact = (a: string, b: string) => a === b;
/** Blank means none; otherwise emails compare case-insensitively (`sameEmail`, as the server normalises owners). */
const email = (a: string, b: string) => (!a.trim() || !b.trim() ? a.trim() === b.trim() : sameEmail(a, b));

/** The fields of `d` that differ from `from` under `same`, in the form's key order. */
export function changed<T extends Record<string, string>>(same: Same<T>, from: T, d: T): (keyof T)[] {
	return (Object.keys(same) as (keyof T)[]).filter((k) => !same[k](from[k], d[k]));
}

/** After a conflict: a field the user changed (under the editor's own `same`) keeps the draft; every other field takes
 *  the reloaded value, so a retry never reverts a colleague's change to a field the user left alone, even one the user
 *  only re-spaced or re-cased. */
export function mergeDraft<T extends Record<string, string>>(same: Same<T>, from: T, draft: T, server: T): T {
	const mine = new Set(changed(same, from, draft));
	const out = { ...server };
	for (const k of mine) out[k] = draft[k];
	return out;
}

export type InitiativeDetails = { title: string; target_month: string; owner: string };
export const INITIATIVE_SAME: Same<InitiativeDetails> = { title: trimmed, target_month: trimmed, owner: email };

export function initiativeDetails(i: Initiative): InitiativeDetails {
	return { title: i.title, target_month: i.target_month ?? '', owner: i.owner ?? '' };
}

/** Only the fields changed since `from` are sent; the month is validated only when it changed. */
export function initiativeEdit(from: InitiativeDetails, d: InitiativeDetails): Edit<Omit<EditItem, 'version'>> {
	if (!d.title.trim()) return { kind: 'invalid', error: BLANK_TITLE };
	const body: Omit<EditItem, 'version'> = {};
	for (const k of changed(INITIATIVE_SAME, from, d)) {
		const v = d[k].trim() || null;
		if (k === 'title') body.title = v!;
		if (k === 'target_month') {
			if (v !== null && !validMonth(v)) return { kind: 'invalid', error: BAD_MONTH };
			body.target_month = v;
		}
		if (k === 'owner') body.owner = v;
	}
	return Object.keys(body).length ? { kind: 'send', body } : { kind: 'unchanged' };
}

export type EpicDetails = { title: string; initiative: string };
export const EPIC_SAME: Same<EpicDetails> = { title: trimmed, initiative: exact };

export function epicDetails(e: Epic): EpicDetails {
	return { title: e.title, initiative: e.initiative ?? '' };
}

export function epicEdit(from: EpicDetails, d: EpicDetails): Edit<Omit<EditItem, 'version'>> {
	if (!d.title.trim()) return { kind: 'invalid', error: BLANK_TITLE };
	const body: Omit<EditItem, 'version'> = {};
	for (const k of changed(EPIC_SAME, from, d)) {
		if (k === 'title') body.title = d.title.trim();
		if (k === 'initiative') body.initiative = d.initiative || null;
	}
	return Object.keys(body).length ? { kind: 'send', body } : { kind: 'unchanged' };
}

/** `due` is `YYYY-MM-DD`, or '' for none; `reversibility` is '' while unset (kb/ADR-0013 R1). */
export type DecisionDraft = { title: string; decider: string; due: string; reversibility: Reversibility | '' };
export const DECISION_SAME: Same<DecisionDraft> = { title: trimmed, decider: trimmed, due: exact, reversibility: exact };

export function decisionDraft(d: RequiredDecision): DecisionDraft {
	return { title: d.title, decider: d.decider, due: d.due ?? '', reversibility: d.reversibility ?? '' };
}

export function decisionEdit(from: DecisionDraft, d: DecisionDraft): Edit<Omit<EditDecision, 'version'>> {
	if (!d.title.trim() || !d.decider.trim()) return { kind: 'invalid', error: 'Title and decider must not be blank.' };
	const body: Omit<EditDecision, 'version'> = {};
	for (const k of changed(DECISION_SAME, from, d)) {
		if (k === 'title') body.title = d.title.trim();
		if (k === 'decider') body.decider = d.decider.trim();
		if (k === 'due') body.due = d.due || null;
		if (k === 'reversibility') body.reversibility = d.reversibility || null;
	}
	return Object.keys(body).length ? { kind: 'send', body } : { kind: 'unchanged' };
}
