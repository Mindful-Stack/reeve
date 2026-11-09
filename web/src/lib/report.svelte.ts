// web/src/lib/report.svelte.ts: the header Report dialog's draft, kept outside the routes (the goalPicker.svelte.ts
// pattern) so closing the dialog, Escape, viewing a similar story or a sign-in redirect never loses it. Context is
// captured once, when a draft starts (spec: Report from anywhere, Rules 3 and 5).
import type { Kind } from '#lib/board.ts';
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
export const isUntouched = (d: string): boolean => d.trim() === '' || Object.values(TEMPLATES).some((t) => t !== '' && d === t);

export const hasDraft = (): boolean => !!report.draft && (report.draft.title.trim() !== '' || !isUntouched(report.draft.description));

/** Opens the dialog. A draft with content, or whose send is uncertain, keeps its context (Rule 3); an empty one is
 *  replaced, so the page the reporter is on now decides origin and chip. */
export function openReport(ctx: Origin & { epic: EpicRef | null }): void {
	if (!report.draft || (!hasDraft() && !report.draft.uncertain)) {
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
	if (!opts.keepStorage) {
		try {
			sessionStorage.removeItem(STORAGE);
		} catch {
			/* storage unavailable: nothing to clear */
		}
	}
}

/** The create body, without the idempotency key. An untouched template is sent as empty; the origin is appended. */
export function payloadOf(d: Draft) {
	const body = isUntouched(d.description) ? '' : d.description.trimEnd();
	const from = `Reported from: [${d.origin.label}](${d.origin.path})`;
	return {
		title: d.title.trim(),
		kind: d.kind as Kind,
		epic: d.epic?.key ?? null,
		description: body ? `${body}\n\n${from}` : from,
		criteria: [] as string[],
		labels: [] as string[]
	};
}

export function persist(email: string): void {
	try {
		if (report.draft) sessionStorage.setItem(STORAGE, JSON.stringify({ email, draft: report.draft }));
	} catch {
		/* storage unavailable: the draft lives until the tab reloads */
	}
}

/** Restores a draft saved for `email`; a draft saved by another account is dropped. */
export function restoreDraft(email: string): boolean {
	try {
		const raw = sessionStorage.getItem(STORAGE);
		if (!raw) return false;
		const saved = JSON.parse(raw) as { email: string; draft: Draft };
		if (saved.email !== email) {
			sessionStorage.removeItem(STORAGE);
			return false;
		}
		report.draft = saved.draft;
		return true;
	} catch {
		return false;
	}
}
