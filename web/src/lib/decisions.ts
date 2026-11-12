// Pure helpers for required decisions: the check label, the "mine" and "closed" filters, due dates and item links.
import type { ClosureKind, NeededIn, NeededInReason, NextStep, QueueLane, RequiredDecision, Reversibility } from '#lib/api/types.ts';
import { age, dayLabel, plural } from '#lib/format.ts';
import { itemRoute } from '#lib/urgency.ts';

export type CheckResult = RequiredDecision['check'];

/** A Record over `CheckResult['kind']` whose entry for each kind takes that variant, so the label can read its fields.
 *  `satisfies` makes `npm run check` fail when the server adds a kind that is not listed here. */
type Labels = { [K in CheckResult['kind']]: (c: Extract<CheckResult, { kind: K }>) => string };

const LABELS = {
	unchecked: () => 'unchecked',
	proposed_pr_open: () => 'proposed (PR linked)',
	accepted: (c) => `accepted ${shortSha(c.commit)}`,
	not_found: () => 'not found',
	// Any other status is shown as the checker reported it ("id mismatch", "unparsable front matter", "rejected", …).
	not_accepted: (c) => (c.superseded_by ? `superseded by ${c.superseded_by}, re-point` : c.status)
} satisfies Labels;

/** The check result as the decisions view shows it. */
export function checkLabel(check: CheckResult): string {
	// The map is keyed by the discriminant, so the entry for check.kind takes exactly check's variant.
	return (LABELS[check.kind] as (c: CheckResult) => string)(check);
}

/** "no record" until a record is linked (nothing can be checked before that), otherwise the check result. */
export function decisionStatus(d: Pick<RequiredDecision, 'check' | 'record'>): string {
	return d.record ? checkLabel(d.check) : 'no record';
}

/** The first seven characters of a commit sha. */
export function shortSha(commit: string): string {
	return commit.slice(0, 7);
}

/** Emails compare case-insensitively and without surrounding whitespace. */
export function sameEmail(a: string, b: string): boolean {
	return a.trim().toLowerCase() === b.trim().toLowerCase();
}

export type DecisionFilter = { mine?: string; closed?: boolean };

/** `mine` is the caller's email (the decider must match it); closed decisions are kept only when `closed`. */
export function filterDecisions(list: RequiredDecision[], f: DecisionFilter): RequiredDecision[] {
	return list.filter((d) => (f.closed || !d.closed) && (!f.mine || sameEmail(d.decider, f.mine)));
}

/** Today as `YYYY-MM-DD` in the browser's time zone, the format of a decision's `due`. */
export function today(now: Date = new Date()): string {
	const p = (n: number) => String(n).padStart(2, '0');
	return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

/** True when the due date has passed; a decision due today is not overdue yet. */
export function overdue(due: string | null | undefined, on: string = today()): boolean {
	return !!due && due < on;
}

/** Where a decision's item opens: the initiative or epic page, or the board's story drawer. */
export function itemHref(key: string, initiatives: ReadonlySet<string>, epics: ReadonlySet<string>): string {
	const k = encodeURIComponent(key);
	if (initiatives.has(key)) return `/initiatives/${k}`;
	if (epics.has(key)) return `/epics/${k}`;
	return `/board?story=${k}`;
}

/** Drops, in place, the expanded ids that are no longer among the listed decisions, so a removed row's thread is not refetched. */
export function pruneExpanded(expanded: Set<number>, listed: readonly { id: number }[]): void {
	const present = new Set(listed.map((d) => d.id));
	for (const id of expanded) if (!present.has(id)) expanded.delete(id);
}

/** Open and unsatisfied: what readiness waits on (kb/ADR-0018 R3). Closed ones are ignored; an accepted record
 *  satisfies its decision even though the decision stays open under check. */
export function blocking(list: RequiredDecision[]): RequiredDecision[] {
	return list.filter((d) => !d.closed && d.check.kind !== 'accepted');
}

/** The closes a decision offers (kb/ADR-0013 R3): a one-way decision is decided by its accepted record, so it can only be
 *  withdrawn; two-way and unset (which behaves as two-way, R1) can be decided or withdrawn. */
export function closeActions(r: Reversibility | null | undefined): ClosureKind[] {
	return r === 'one_way' ? ['withdrawn'] : ['decided', 'withdrawn'];
}

export type DateLine = { text: string; tone: 'normal' | 'warning' | 'danger' };

/** A `YYYY-MM-DD` value as a local midnight, so day differences ignore the time zone's offset. */
export function localDate(s: string): Date {
	const [y, m, d] = s.split('-').map(Number);
	return new Date(y, m - 1, d);
}

export function daysBetween(from: string, to: string): number {
	return Math.round((localDate(to).getTime() - localDate(from).getTime()) / 86_400_000);
}

const days = (n: number): string => plural(n, 'day', 'days');

/** The decision's date line from the viewer's date `on`: due in n days, due today or overdue (with the weekday), or, with
 *  no date, how long it has been open. */
export function dateLine(d: Pick<RequiredDecision, 'due' | 'created_at'>, on: string = today()): DateLine {
	if (!d.due) {
		const created = new Date(d.created_at);
		// The timestamp's local date, like `on`, so the age does not shift near midnight; "?" as age() shows it when unparseable.
		const open = Number.isNaN(created.getTime()) ? '?' : days(Math.max(0, daysBetween(today(created), on)));
		return { text: `No date · open ${open}`, tone: 'warning' };
	}
	const n = daysBetween(on, d.due);
	if (n === 0) return { text: 'Due today', tone: 'warning' };
	const when = localDate(d.due)
		.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' })
		.replace(',', '');
	return n > 0
		? { text: `Due in ${days(n)} (${when})`, tone: 'normal' }
		: { text: `Overdue ${days(-n)} (${when})`, tone: 'danger' };
}

/** The record's own reversibility when it disagrees with the decision's, else null: a warning only, and only when both are
 *  set and differ (kb/ADR-0013 R2). An id mismatch carries no value, so it never warns. */
export function recordMismatch(d: Pick<RequiredDecision, 'reversibility' | 'check'>): Reversibility | null {
	const theirs = 'reversibility' in d.check ? d.check.reversibility : null;
	return d.reversibility && theirs && theirs !== d.reversibility ? theirs : null;
}

/** A decision's reversibility as the lists and the drawer say it. */
export const REVERSIBILITY_TEXT: Record<Reversibility, string> = { one_way: 'One-way', two_way: 'Two-way' };

/** The queue's lanes, top to bottom. */
export const LANE_ORDER: QueueLane[] = ['review', 'this_sprint', 'next_sprint', 'beyond', 'open', 'satisfied', 'closed'];

export const LANE_LABEL: Record<QueueLane, string> = {
	review: 'Record to review',
	this_sprint: 'Needed this sprint',
	next_sprint: 'Needed next sprint',
	beyond: 'Needed after next sprint',
	open: 'Open, not blocking yet',
	satisfied: 'Accepted',
	closed: 'Closed'
};

/** The anchors of the lanes that have one on the decisions page. */
export const LANE_ANCHOR: Partial<Record<QueueLane, string>> = { review: 'review', this_sprint: 'this-sprint', next_sprint: 'next-sprint' };

/** What an open, unsatisfied decision needs next, as one line. `reversibility` decides whether closing is offered. */
export function nextStepText(s: NextStep, reversibility: Reversibility | null | undefined, now: number = Date.now()): string {
	switch (s.kind) {
		case 'write_record':
			return reversibility === 'one_way' ? 'Write a record' : 'Write a record, or close it';
		case 'configure_repo':
			return `Repository ${s.repo} is not checked: an admin adds it to the server's repos`;
		case 'fix_record':
			return `Fix the record: ${checkLabel(s.result)}`;
		case 'review_record':
			return 'Linked PR \u00b7 not accepted yet';
		case 'await_check':
			return `Waiting for the check (${s.checked_at ? `last ran ${age(s.checked_at, now)} ago` : 'never ran'})`;
	}
}

/** Why the decision is needed when it is: the goal's outcome, the discovery that waits on it, or the promise of an
 *  affected story (the item holding it, the effective date, and whether it is overdue). A reason added after this
 *  client was built reads `unknown`. */
export function neededInText(r: NeededInReason): string {
	switch (r.kind) {
		case 'goal':
			return `for goal: ${r.outcome}`;
		case 'discovery':
			return `${r.key} is in discovery`;
		case 'promise':
			return `promised by ${dayLabel(r.on)}${r.overdue ? ', overdue' : ''}: ${r.key}`;
		case 'unknown':
			return 'needed (reason not shown)';
		default: {
			const never: never = r;
			return never;
		}
	}
}

/** Where the item a reason names opens: an initiative or epic page, or a story's drawer over `url`'s page. Null for a
 *  reason that names no item (a goal, or one this client does not know). */
export function reasonHref(r: NeededInReason, url: { readonly href: string }): string | null {
	switch (r.kind) {
		case 'discovery':
		case 'promise':
			return itemRoute({ item: r.level, key: r.key }, url);
		case 'goal':
		case 'unknown':
			return null;
		default: {
			const never: never = r;
			return never;
		}
	}
}

/** The sprint a decision is needed in. Shown beside `neededInText`, so a row in the review lane still says which sprint. */
export function neededLabel(n: NeededIn): string {
	switch (n) {
		case 'this_sprint':
			return 'Needed this sprint';
		case 'next_sprint':
			return 'Needed next sprint';
		case 'beyond':
			return 'Needed after next sprint';
		case 'none':
			return 'Not needed yet';
	}
}
