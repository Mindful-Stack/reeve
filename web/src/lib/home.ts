// Home's partitions of the lists it reads: which decisions wait on me, what I have started, and what to pick next. All
// display: the server decides readiness, lanes and ownership, and these lists arrive filtered by the server, so nothing
// here filters a backlog (kb/ADR-0021 R1).
import type { DecisionQueueEntry, Sprint, StoryWithReadiness } from '#lib/api/types.ts';
import { daysBetween, sameEmail } from '#lib/decisions.ts';
import { ACTIVE } from '#lib/sprint.ts';

/** My decisions in the queue's order: listed when needed this or next sprint; those with a record to review, and those
 *  needed later (beyond the next sprint, or not yet blocking), are only counted. */
export function myDecisions(entries: DecisionQueueEntry[], me: string) {
	const mine = entries.filter((e) => sameEmail(e.decision.decider, me));
	return {
		listed: mine.filter((e) => e.lane === 'this_sprint' || e.lane === 'next_sprint'),
		review: mine.filter((e) => e.lane === 'review').length,
		later: mine.filter((e) => e.lane === 'beyond' || e.lane === 'open').length
	};
}

/** My unfinished stories: the started ones (in progress, in review, testing) in the server's order, and how many are
 *  still todo and how many of those are ready. */
export function underWay(mine: StoryWithReadiness[]) {
	const todo = mine.filter((s) => s.story.delivery === 'todo');
	return {
		started: mine.filter((s) => ACTIVE.has(s.story.delivery)),
		todo: todo.length,
		ready: todo.filter((s) => s.readiness.ready).length
	};
}

const urgent = (s: StoryWithReadiness) => s.story.urgent_sources.length > 0;
const servesGoal = (s: StoryWithReadiness) => s.story.goals.length > 0;

/** Ready todo stories nobody owns: urgent first, then those serving a sprint goal, then the rest, each group in the
 *  server's rank order (filters, never a sort); the first `limit` and how many there are. Urgency orders this list only:
 *  it is not a sprint view (kb/ADR-0017 R5). */
export function pickNext(ready: StoryWithReadiness[], limit = 5) {
	const free = ready.filter((s) => s.story.owner == null);
	const ordered = [
		...free.filter(urgent),
		...free.filter((s) => !urgent(s) && servesGoal(s)),
		...free.filter((s) => !urgent(s) && !servesGoal(s))
	];
	return { shown: ordered.slice(0, limit), total: ordered.length };
}

/** A `YYYY-MM-DD` date at local midnight. */
function local(d: string): Date {
	const [y, m, n] = d.split('-').map(Number);
	return new Date(y, m - 1, n);
}
const dayMonth = (d: string) => local(d).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });
const days = (n: number) => `${n} ${n === 1 ? 'day' : 'days'}`;

/** The active sprint named by its dates, as the sprint page names it, and how long is left from the viewer's date `on`.
 *  An active sprint runs until it is closed (kb/ADR-0014 R1), so past its end date it has not ended. */
export function sprintLine(s: Pick<Sprint, 'starts_on' | 'ends_on'>, on: string): string {
	const sameMonth = local(s.starts_on).getMonth() === local(s.ends_on).getMonth();
	const range = `${sameMonth ? local(s.starts_on).getDate() : dayMonth(s.starts_on)}–${dayMonth(s.ends_on)}`;
	const n = daysBetween(on, s.ends_on);
	const when = n > 0 ? `${days(n)} left` : n === 0 ? 'ends today' : `end date passed ${days(-n)} ago`;
	return `Sprint ${range} · ${when}`;
}
