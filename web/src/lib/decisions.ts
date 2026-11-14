// Pure helpers for required decisions: the check label, the "mine" and "closed" filters, due dates and item links.
import type {
	ClosureKind,
	DecisionQueue,
	DecisionQueueEntry,
	DiscoveryEntry,
	NeededInReason,
	NextStep,
	QueueLane,
	RequiredDecision,
	Reversibility
} from '#lib/api/types.ts';
import { age, dayLabel, month, plural } from '#lib/format.ts';
import { decisionHref } from '#lib/drawer.ts';
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

/** The queue page's lanes: the wire's `beyond` splits into Now (a Now reason) and Later. */
export type PageLane = 'this_sprint' | 'next_sprint' | 'now' | 'later' | 'open' | 'satisfied' | 'closed';

/** The queue page's lanes, top to bottom (spec §2). */
export const LANE_ORDER: PageLane[] = ['this_sprint', 'next_sprint', 'now', 'later', 'open', 'satisfied', 'closed'];

export const LANE_LABEL: Record<PageLane, string> = {
	this_sprint: 'Needed this sprint',
	next_sprint: 'Needed next sprint',
	now: 'Now, no goal yet',
	later: 'Later',
	open: 'Open, not blocking yet',
	satisfied: 'Accepted',
	closed: 'Closed'
};

/** The anchors of the lanes that have one on the queue page. */
export const LANE_ANCHOR: Partial<Record<PageLane, string>> = { this_sprint: 'this-sprint', next_sprint: 'next-sprint' };

/** The page lane of a wire lane and its reason: `beyond` is Now when it carries a reason (Later never does, a server
 *  invariant). `review` is no longer sent; an older server's reads as Open. */
export function pageLane(lane: QueueLane, reason: NeededInReason | null | undefined): PageLane {
	if (lane === 'beyond') return reason != null ? 'now' : 'later';
	return lane === 'review' ? 'open' : lane;
}

/** One top-level row of the queue page: a standalone decision, or a discovery row with the decisions nested under it. */
export type QueueRow = { kind: 'decision'; entry: DecisionQueueEntry } | DiscoveryRow;
/** A discovery row with the decisions nested under it. */
export type DiscoveryRow = { kind: 'discovery'; row: DiscoveryEntry; nested: DecisionQueueEntry[] };

/** The page's top-level rows per lane, in server order (nothing is re-sorted): each discovery row goes before
 *  `entries[at]` (`discoveries` arrive in ascending `at`, as the server emits them), a decision with `under` goes inside
 *  the row it names (one the response lacks leaves it standing alone), and `beyond` splits into Now and Later. With
 *  `me`, a standalone decision is kept when `me` decides it and a row when it nests a decision `me` decides; a kept row
 *  keeps its whole nested list (the list renders only the decisions the page passes it), so its reason can name any of
 *  them. Closed and satisfied decisions never nest. */
export function queueRows(q: Pick<DecisionQueue, 'entries' | 'discoveries'>, f: { me?: string } = {}): Map<PageLane, QueueRow[]> {
	const out = new Map<PageLane, QueueRow[]>();
	const lane = (l: PageLane): QueueRow[] => {
		let rows = out.get(l);
		if (!rows) out.set(l, (rows = []));
		return rows;
	};
	const byKey = new Map<string, DiscoveryRow>();
	const mine = (e: DecisionQueueEntry) => !f.me || sameEmail(e.decision.decider, f.me);
	let next = 0;
	const rowsBefore = (i: number) => {
		for (; next < q.discoveries.length && q.discoveries[next].at <= i; next++) {
			const d = q.discoveries[next];
			const r = { kind: 'discovery' as const, row: d, nested: [] as DecisionQueueEntry[] };
			byKey.set(d.key, r);
			lane(pageLane(d.lane, d.needed_in_reason)).push(r);
		}
	};
	q.entries.forEach((e, i) => {
		rowsBefore(i);
		const parent = e.under ? byKey.get(e.under) : undefined;
		if (parent) parent.nested.push(e);
		else if (mine(e)) lane(pageLane(e.lane, e.needed_in_reason)).push({ kind: 'decision', entry: e });
	});
	rowsBefore(Infinity);
	if (!f.me) return out;
	return new Map([...out].map(([l, rows]) => [l, rows.filter((r) => r.kind === 'decision' || r.nested.some(mine))]));
}

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

/** A reason phrase split around the item key it names, so a view can make the key a link. */
export type ReasonParts = { before: string; key: string; after: string };

/** A queue row's reason as one line, split around the key it links (`reasonHref`; `key` is empty when it links
 *  nothing). A Now reason (spec §3) reads off its row (what it holds, the date of the decision it names) after
 *  "Now · "; the others name the goal's outcome, the discovery that waits on a decision, or the promise of an affected
 *  story (the item holding it, the effective date, and whether it is overdue). A reason added after this client was
 *  built reads `unknown`, and so does any kind the generated types do not list. */
export function reasonLine(r: NeededInReason, row: QueueRow): ReasonParts {
	const text = (before: string): ReasonParts => ({ before, key: '', after: '' });
	const lists = row.kind === 'decision' ? row.entry : row.row;
	switch (r.kind) {
		case 'goal':
			return text(`for goal: ${r.outcome}`);
		case 'discovery':
			return { before: '', key: r.key, after: ' is in discovery' };
		case 'promise':
			return { before: `promised by ${dayLabel(r.on)}${r.overdue ? ', overdue' : ''}: `, key: r.key, after: '' };
		case 'urgent':
			return { before: 'Now · urgent via ', key: r.key, after: '' };
		case 'target_month':
			return text(`Now · target ${month(r.month)}`);
		case 'decision_due': {
			// A standalone decision's reason names itself; a row's names one of its nested decisions (kept whole under Mine).
			const own = row.kind === 'decision';
			const d = own ? row.entry.decision : row.nested.find((e) => e.decision.id === r.id)?.decision;
			const when = d?.due ? ` due ${dayLabel(d.due)}` : '';
			return own ? text(`Now ·${when}`) : { before: 'Now · ', key: `decision #${r.id}`, after: when };
		}
		case 'held': {
			const held = lists.blocks.length;
			return text(held ? `Now · holds ${held}` : `Now · holds none, ${lists.under_way.length} under way`);
		}
		case 'unknown':
			return text('needed (reason not shown)');
		default: {
			// A kind newer than the generated types: compile-time exhaustive, harmless at run time.
			r satisfies never;
			return text('needed (reason not shown)');
		}
	}
}

/** Where the item a reason names opens: an initiative or epic page, or a story's drawer over `url`'s page; for a Now
 *  `decision_due`, that decision's drawer over `url`'s page. Null for a reason that names nothing to open. */
export function reasonHref(r: NeededInReason, url: { readonly href: string }): string | null {
	switch (r.kind) {
		case 'discovery':
		case 'promise':
		case 'urgent':
			return itemRoute({ item: r.level, key: r.key }, url);
		case 'decision_due':
			return decisionHref(url, r.id);
		case 'goal':
		case 'target_month':
		case 'held':
		case 'unknown':
			return null;
		default: {
			r satisfies never;
			return null;
		}
	}
}

/** The sprint a decision is needed in outside the queue page (Home, item pages, the drawer), read off its lane, which
 *  is what the page, the banner and Home count, not off its own window; a decision nested under a discovery row adds
 *  "in discovery" (no key: the header already links the item). Null when its lane says nothing of a sprint (Now says
 *  it in its reason). */
export function neededLabel(e: Pick<DecisionQueueEntry, 'lane' | 'needed_in_reason' | 'under'>): string | null {
	const sprint = { this_sprint: 'Needed this sprint', next_sprint: 'Needed next sprint' } as const;
	// A nested decision on a `beyond` row sits in Now (rows never sit in Later), whatever its own window says.
	const later = e.lane === 'beyond' && !e.under && pageLane(e.lane, e.needed_in_reason) === 'later';
	const base = e.lane === 'this_sprint' || e.lane === 'next_sprint' ? sprint[e.lane] : later ? 'Needed later' : null;
	return [base, e.under ? 'in discovery' : null].filter(Boolean).join(' · ') || null;
}

/** What a queue row holds: "Holds N stories · M under way", or "M under way" when it holds none. */
export function holdsLabel(blocks: readonly unknown[], underWay: readonly unknown[]): string {
	const away = `${underWay.length} under way`;
	return blocks.length ? `Holds ${plural(blocks.length, 'story', 'stories')}${underWay.length ? ` · ${away}` : ''}` : away;
}
