import type { Confidence, GoalProgress, SprintCounts, SprintWarning, Story, Verdict } from '#lib/api/types.ts';
import { dayLabel } from '#lib/format.ts';

/** kb/ADR-0014 R2, shown wherever counts are (the CLI's `MEMBERSHIP_RULE`). */
export const RULE = 'Counts the work completed in this sprint; work started but not finished shows as in flight.';

export const ACTIVE = new Set<Story['delivery']>(['in_progress', 'in_review', 'testing']);

/** The start nudge: a move from todo or done into an active state, while a sprint is active, whose reply
 *  says the story serves no goal. `after` is the server's story from the move's reply, never the card before it. */
export function shouldNudge(from: Story['delivery'], after: Pick<Story, 'delivery' | 'goals'>, sprintActive: boolean): boolean {
	return sprintActive && (from === 'todo' || from === 'done') && ACTIVE.has(after.delivery) && after.goals.length === 0;
}

/** Whether an open nudge still asks a question: after a reload, the story (the server's, as reloaded) is still in an
 *  active state and still serves no goal. Linked meanwhile, or moved on, the bar goes. */
export function nudgeStillDue(s: Pick<Story, 'delivery' | 'goals'>): boolean {
	return ACTIVE.has(s.delivery) && s.goals.length === 0;
}

/** The board's `?goal=` value as a filter: a goal of the active sprint, `'none'` (upkeep), or null. Without an active
 *  sprint, or for an id that is not one of its goals, nothing is filtered: the strip that could clear it is not there. */
export function goalFilter(param: string | null, activeGoalIds: number[] | null): number | 'none' | null {
	if (!activeGoalIds || !param) return null;
	if (param === 'none') return 'none';
	const id = /^\d+$/.test(param) ? Number(param) : NaN;
	return activeGoalIds.includes(id) ? id : null;
}

export function upkeepText(c: Pick<SprintCounts, 'completed' | 'upkeep_completed'>, line: number): string {
	if (c.completed === 0) return `nothing completed yet (team line ${line}%)`;
	const pct = Math.round((100 * c.upkeep_completed) / c.completed);
	return `${c.upkeep_completed} of ${c.completed} completed stories were upkeep (${pct}%; team line ${line}%)`;
}

/** `blocked_decisions` is deliberately absent: it is a live measure, and a closed sprint shows the closed entry's value
 *  only, so a difference in it is not "changed since close" (the CLI's `counts_differ` ignores it too). */
const GOAL_FIELDS = ['completed', 'in_flight', 'todo', 'in_review'] as const satisfies readonly (keyof GoalProgress)[];

/** kb/ADR-0014 R6: show "at close" beside "now" only when they differ; `atClose` is the `closed` entry's counts.
 *  Compares the numbers field by field, goals keyed by `goal_id`: never serialised text, whose key and array order
 *  says nothing about the counts. */
export function countsDiffer(now: SprintCounts, atClose: SprintCounts | null): boolean {
	if (atClose === null) return false;
	if (now.completed !== atClose.completed || now.upkeep_completed !== atClose.upkeep_completed) return true;
	if (now.goals.length !== atClose.goals.length) return true;
	const then = new Map(atClose.goals.map((g) => [g.goal_id, g]));
	return now.goals.some((g) => {
		const t = then.get(g.goal_id);
		return !t || GOAL_FIELDS.some((f) => g[f] !== t[f]);
	});
}

const CONFIDENCE_LABEL: Record<Confidence, string> = {
	on_track: 'on track',
	at_risk: 'at risk',
	off_track: 'off track'
};

export const confidenceLabel = (c: Confidence): string => CONFIDENCE_LABEL[c];

/** `text` cut to at most `max` characters, the last an ellipsis when cut (show the full text as a `title`). */
export function truncate(text: string, max: number): string {
	return text.length <= max ? text : `${text.slice(0, max - 1)}…`;
}

/** A goal's confidence dot tone (the `--ready`, `--warn`, `--danger` tokens); muted when none is recorded. */
export function confidenceTone(c: Confidence | null | undefined): 'ready' | 'warn' | 'danger' | 'muted' {
	return c === 'on_track' ? 'ready' : c === 'at_risk' ? 'warn' : c === 'off_track' ? 'danger' : 'muted';
}

/** A window number in a closed sprint's view: the derived value alone, or both when the `closed` entry's value differs
 *  (kb/ADR-0014 R6; the CLI's `now_then`). */
export function nowThen(now: number, atClose: number | undefined): string {
	return atClose === undefined || atClose === now ? String(now) : `now ${now} · at close ${atClose}`;
}

/** "blocked by 1 decision", "blocked by 3 decisions". */
export const blockedText = (n: number): string => `blocked by ${n} decision${n === 1 ? '' : 's'}`;

/** "1 goal", "3 goals". */
export const goalsText = (n: number): string => `${n} ${n === 1 ? 'goal' : 'goals'}`;

/** "1 goal stays", "3 goals stay". */
export const staysText = (n: number): string => `${goalsText(n)} ${n === 1 ? 'stays' : 'stay'}`;

const VERDICT_LABEL: Record<Verdict, string> = { met: 'met', partly_met: 'partly met', missed: 'missed' };
export const verdictLabel = (v: Verdict): string => VERDICT_LABEL[v];

const WARNING_TEXT: Record<SprintWarning, string> = {
	no_goals: 'the sprint has no goals',
	more_than_three_goals: 'more than three goals thins focus'
};
/** Advice from the goal count, in the CLI's words. */
export const warningText = (w: SprintWarning): string => WARNING_TEXT[w];

/** The upkeep line of a closed sprint: `upkeepText` while its totals match the `closed` entry's (or there is none),
 *  else the derived sentence first with the totals at close in one parenthesis. */
export function upkeepTextAt(
	now: Pick<SprintCounts, 'completed' | 'upkeep_completed'>,
	atClose: Pick<SprintCounts, 'completed' | 'upkeep_completed'> | null,
	line: number
): string {
	if (!atClose || (atClose.completed === now.completed && atClose.upkeep_completed === now.upkeep_completed))
		return upkeepText(now, line);
	return `${now.upkeep_completed} of ${now.completed} completed stories were upkeep (at close: ${atClose.upkeep_completed} of ${atClose.completed}) · team line ${line}%`;
}

/** The ISO day after `iso` (calendar arithmetic only, no clock). */
function nextDay(iso: string): string {
	const d = new Date(`${iso}T00:00:00Z`);
	d.setUTCDate(d.getUTCDate() + 1);
	return d.toISOString().slice(0, 10);
}

/** A closed sprint's promise list heading: "Promises due 11–24 Nov" (the day after the previous planned end to this
 *  one's), "Promises due 28 Oct–10 Nov" across months, "Promises due up to 27 Oct" for the first sprint. */
export function promisesHeading(previous: string | null, endsOn: string): string {
	const end = dayLabel(endsOn);
	if (previous == null) return `Promises due up to ${end}`;
	const from = nextDay(previous);
	const sameMonth = from.slice(0, 7) === endsOn.slice(0, 7);
	return `Promises due ${sameMonth ? String(Number(from.slice(8, 10))) : dayLabel(from)}–${end}`;
}
