// Home's partitions of the lists it reads: which decisions wait on me, what I have started, and what to pick next. All
// display: the server decides readiness, lanes and ownership, and these lists arrive filtered by the server, so nothing
// here filters a backlog (kb/ADR-0021 R1).
import type { DecisionQueueEntry, Sprint, StoryWithReadiness } from '#lib/api/types.ts';
import { daysBetween, localDate, sameEmail } from '#lib/decisions.ts';
import { plural } from '#lib/format.ts';
import { ACTIVE } from '#lib/sprint.ts';

/** My decisions in the queue's order: listed when needed this or next sprint; those with a record to review, and those
 *  beyond the next sprint or not blocking yet, are only counted. */
export function myDecisions(entries: DecisionQueueEntry[], me: string) {
	const mine = entries.filter((e) => sameEmail(e.decision.decider, me));
	return {
		listed: mine.filter((e) => e.lane === 'this_sprint' || e.lane === 'next_sprint'),
		review: mine.filter((e) => e.lane === 'review').length,
		later: mine.filter((e) => e.lane === 'beyond' || e.lane === 'open').length
	};
}

/** My unfinished stories: the started ones (in progress, in review, testing) in rank order (the list arrives ordered by
 *  delivery state first), and how many are still todo and how many of those are ready. */
export function underWay(mine: StoryWithReadiness[]) {
	const todo = mine.filter((s) => s.story.delivery === 'todo');
	return {
		started: mine.filter((s) => ACTIVE.has(s.story.delivery)).sort((a, b) => a.story.rank - b.story.rank),
		todo: todo.length,
		ready: todo.filter((s) => s.readiness.ready).length
	};
}

const urgent = (s: StoryWithReadiness) => s.story.urgent_sources.length > 0;
const servesGoal = (s: StoryWithReadiness) => s.story.goals.length > 0;

/** The first `limit` ready todo stories nobody owns: urgent first, then those serving a sprint goal, then the rest, each
 *  group in the server's rank order (filters, never a sort). Urgency orders this list only: it is not a sprint view
 *  (kb/ADR-0017 R5). */
export function pickNext(ready: StoryWithReadiness[], limit = 5): StoryWithReadiness[] {
	const free = ready.filter((s) => s.story.owner == null);
	return [
		...free.filter(urgent),
		...free.filter((s) => !urgent(s) && servesGoal(s)),
		...free.filter((s) => !urgent(s) && !servesGoal(s))
	].slice(0, limit);
}

const dayMonth = (d: Date) => d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });

/** The active sprint named by its dates ("Sprint 10–24 Nov") and how long is left from the viewer's date `on`. An active
 *  sprint runs until it is closed (kb/ADR-0014 R1), so past its end date it has not ended. */
export function sprintLine(s: Pick<Sprint, 'starts_on' | 'ends_on'>, on: string): string {
	const start = localDate(s.starts_on);
	const end = localDate(s.ends_on);
	const range = `${start.getMonth() === end.getMonth() ? start.getDate() : dayMonth(start)}–${dayMonth(end)}`;
	const n = daysBetween(on, s.ends_on);
	const when = n > 0 ? `${plural(n, 'day', 'days')} left` : n === 0 ? 'ends today' : `end date passed ${plural(-n, 'day', 'days')} ago`;
	return `Sprint ${range} · ${when}`;
}
