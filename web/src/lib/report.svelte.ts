// web/src/lib/report.svelte.ts: the header Report dialog's draft, kept outside the routes (the goalPicker.svelte.ts
// pattern) so closing the dialog, Escape, viewing a similar story or a sign-in redirect never loses it. Context is
// captured once, when a draft starts (spec: Report from anywhere, Rules 3 and 5).
import { KINDS, type Kind } from '#lib/board.ts';
import { newDraftKey, type DraftKey } from '#lib/idempotency.ts';

export type Origin = { path: string; label: string };
export type EpicRef = { key: string; title: string };
export type Draft = {
	title: string;
	kind: Kind | null;
	description: string;
	origin: Origin;
	/** The epic page's epic, until "Send to triage instead"; null means triage places it. */
	epic: EpicRef | null;
	/** The idempotency key and the payload text last sent with it: an uncertain send retries exactly this. */
	key: DraftKey;
	/** The hit the reporter left the dialog from, to refocus on resume. */
	leftFrom: string | null;
	/** The last send's answer was lost (transport failure): it may have been filed. Cleared by a create or Discard. */
	uncertain: boolean;
};

const TEMPLATES: Record<Kind, string> = {
	bug: 'What happened:\n\nWhat I expected:\n\nHow can we reproduce it? (if known)\n',
	story: 'What and why:\n',
	chore: ''
};
const STORAGE = 'reeve.report-draft';

export const report = $state<{ draft: Draft | null; open: boolean }>({ draft: null, open: false });

export const templateFor = (k: Kind): string => TEMPLATES[k];
export const isUntouched = (d: string): boolean =>
	d.trim() === '' || Object.values(TEMPLATES).some((t) => t !== '' && d.trimEnd() === t.trimEnd());

/** A draft counts once something was typed, a type chosen, or a send is uncertain. */
export const hasDraft = (): boolean => {
	const d = report.draft;
	return !!d && (d.uncertain || d.kind !== null || d.title.trim() !== '' || !isUntouched(d.description));
};

const isStr = (x: unknown): x is string => typeof x === 'string';
const isObj = (x: unknown): x is Record<string, unknown> => typeof x === 'object' && x !== null;

/** Guards stored JSON: a draft from an older build must never reach the header. */
function isDraft(x: unknown): x is Draft {
	if (!isObj(x)) return false;
	const { origin, epic, key } = x;
	return (
		isStr(x.title) &&
		isStr(x.description) &&
		(x.kind === null || (isStr(x.kind) && (KINDS as string[]).includes(x.kind))) &&
		isObj(origin) && isStr(origin.path) && isStr(origin.label) &&
		(epic === null || (isObj(epic) && isStr(epic.key) && isStr(epic.title))) &&
		isObj(key) && isStr(key.key) && (key.sent === null || isStr(key.sent)) &&
		(x.leftFrom === null || isStr(x.leftFrom)) &&
		typeof x.uncertain === 'boolean'
	);
}

function clearStored(): void {
	try {
		sessionStorage.removeItem(STORAGE);
	} catch {
		/* storage unavailable: nothing to clear */
	}
}

/** Opens the dialog. A draft with content, or whose send is uncertain, keeps its context (Rule 3); an empty one is
 *  replaced, so the page the reporter is on now decides origin and chip. */
export function openReport(ctx: Origin & { epic: EpicRef | null }): void {
	if (!report.draft || (!hasDraft())) {
		report.draft = {
			title: '',
			kind: null,
			description: '',
			origin: { path: ctx.path, label: ctx.label },
			epic: ctx.epic,
			key: newDraftKey(),
			leftFrom: null,
			uncertain: false
		};
	}
	report.open = true;
}

export function discardDraft(opts: { keepStorage?: boolean } = {}): void {
	report.draft = null;
	report.open = false;
	if (!opts.keepStorage) clearStored();
}

/** The create body, without the idempotency key. An untouched template is sent as empty; the origin is appended. */
export function payloadOf(d: Draft) {
	if (d.kind === null) throw new Error('a report needs a kind');
	const body = isUntouched(d.description) ? '' : d.description.trimEnd();
	const label = d.origin.label.replace(/[[\]\\]/g, '\\$&');
	const path = d.origin.path.replace(/[()\s]/g, (c) => '%' + c.charCodeAt(0).toString(16).toUpperCase().padStart(2, '0'));
	const from = `Reported from: [${label}](${path})`;
	return {
		title: d.title.trim(),
		kind: d.kind,
		epic: d.epic?.key ?? null,
		description: body ? `${body}\n\n${from}` : from,
		criteria: [] as string[],
		labels: [] as string[]
	};
}

/** The one canonical string the dialog passes to `draftKey`. */
export const payloadText = (d: Draft): string => JSON.stringify(payloadOf(d));

export function persist(email: string): void {
	if (!report.draft || !hasDraft()) return clearStored();
	try {
		sessionStorage.setItem(STORAGE, JSON.stringify({ email, draft: report.draft }));
	} catch {
		/* storage unavailable: the draft lives until the tab reloads */
	}
}

/** Restores a draft saved for `email`; another account's, a malformed or an empty one is dropped. */
export function restoreDraft(email: string): boolean {
	try {
		const raw = sessionStorage.getItem(STORAGE);
		if (!raw) return false;
		const saved: unknown = JSON.parse(raw);
		if (isObj(saved) && saved.email === email && isDraft(saved.draft)) {
			const prev = report.draft;
			report.draft = saved.draft;
			if (hasDraft()) return true;
			report.draft = prev;
		}
	} catch {
		/* unreadable: dropped below */
	}
	clearStored();
	return false;
}
