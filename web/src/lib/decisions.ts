// Pure helpers for required decisions: the check label, the "mine" and "closed" filters, due dates and item links.
import type { ClosureKind, RequiredDecision, Reversibility } from '#lib/api/types.ts';

export type CheckResult = RequiredDecision['check'];

/** A Record over `CheckResult['kind']` whose entry for each kind takes that variant, so the label can read its fields.
 *  `satisfies` makes `npm run check` fail when the server adds a kind that is not listed here. */
type Labels = { [K in CheckResult['kind']]: (c: Extract<CheckResult, { kind: K }>) => string };

const LABELS = {
	unchecked: () => 'unchecked',
	proposed_pr_open: () => 'proposed, PR open',
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

/** Open and unsatisfied: what readiness waits on (kb/ADR-0003 R3). Closed ones are ignored; an accepted record
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
function localDate(s: string): Date {
	const [y, m, d] = s.split('-').map(Number);
	return new Date(y, m - 1, d);
}

function daysBetween(from: string, to: string): number {
	return Math.round((localDate(to).getTime() - localDate(from).getTime()) / 86_400_000);
}

function plural(n: number): string {
	return `${n} ${n === 1 ? 'day' : 'days'}`;
}

/** The decision's date line from the viewer's date `on`: due in n days, due today or overdue (with the weekday), or, with
 *  no date, how long it has been open. */
export function dateLine(d: Pick<RequiredDecision, 'due' | 'created_at'>, on: string = today()): DateLine {
	if (!d.due) {
		const created = new Date(d.created_at);
		// The timestamp's local date, like `on`, so the age does not shift near midnight; "?" as age() shows it when unparseable.
		const open = Number.isNaN(created.getTime()) ? '?' : plural(Math.max(0, daysBetween(today(created), on)));
		return { text: `No date · open ${open}`, tone: 'warning' };
	}
	const n = daysBetween(on, d.due);
	if (n === 0) return { text: 'Due today', tone: 'warning' };
	const when = localDate(d.due)
		.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' })
		.replace(',', '');
	return n > 0
		? { text: `Due in ${plural(n)} (${when})`, tone: 'normal' }
		: { text: `Overdue ${plural(-n)} (${when})`, tone: 'danger' };
}

/** The record's own reversibility when it disagrees with the decision's, else null: a warning only, and only when both are
 *  set and differ (kb/ADR-0013 R2). An id mismatch carries no value, so it never warns. */
export function recordMismatch(d: Pick<RequiredDecision, 'reversibility' | 'check'>): Reversibility | null {
	const theirs = 'reversibility' in d.check ? d.check.reversibility : null;
	return d.reversibility && theirs && theirs !== d.reversibility ? theirs : null;
}
