// Pure helpers for the initiative page: per-epic story counts, the epic card's status line and the
// decisions the status strip names. Readiness and blocking come from server data only (kb/ADR-0018 R1/R3).
import type { Epic, Initiative, RequiredDecision, StoryWithReadiness } from '#lib/api/types.ts';
import { blocking } from '#lib/decisions.ts';

export type EpicCounts = { total: number; done: number; inProgress: number; readyTodo: number; needCriteria: number };

const zero = (): EpicCounts => ({ total: 0, done: 0, inProgress: 0, readyTodo: 0, needCriteria: 0 });

/** True for a todo story the server holds back for want of acceptance criteria: past todo, the gate is behind it. */
export function needsCriteria(s: StoryWithReadiness): boolean {
	return s.story.delivery === 'todo' && s.readiness.reasons.some((r) => r.reason === 'no_criteria');
}

/** Counts per epic key, every listed epic present (zeros when it has no stories); stories of other epics are ignored. */
export function epicCounts(stories: StoryWithReadiness[], epicKeys: Iterable<string>): Map<string, EpicCounts> {
	const out = new Map<string, EpicCounts>();
	for (const k of epicKeys) out.set(k, zero());
	for (const s of stories) {
		const c = s.story.epic ? out.get(s.story.epic) : undefined;
		if (!c) continue;
		c.total++;
		const d = s.story.delivery;
		if (d === 'done') c.done++;
		else if (d === 'todo') {
			if (s.readiness.ready) c.readyTodo++;
		} else c.inProgress++;
		if (needsCriteria(s)) c.needCriteria++;
	}
	return out;
}

/** The counts of several epics added up: the initiative's progress and criteria totals. */
export function sumCounts(counts: Iterable<EpicCounts>): EpicCounts {
	const t = zero();
	for (const c of counts) {
		t.total += c.total;
		t.done += c.done;
		t.inProgress += c.inProgress;
		t.readyTodo += c.readyTodo;
		t.needCriteria += c.needCriteria;
	}
	return t;
}

/** An epic card's status line: the blocking decision it waits on, else stories needing criteria, else the flow. */
export function epicStatus(c: EpicCounts, waitingOn: string | null): string {
	if (waitingOn) return `Waiting on ${waitingOn}`;
	if (c.needCriteria) return `${c.needCriteria} ${c.needCriteria === 1 ? 'needs' : 'need'} criteria`;
	if (c.total === 0) return 'No stories yet';
	if (c.done === c.total) return 'All done';
	return `${c.readyTodo} ready · ${c.inProgress} in progress`;
}

/** What an epic card says it waits on: the first open, unsatisfied decision on the epic itself, else the first one on
 *  its initiative (which holds up every epic under it), else null. blocking() keeps the server's order. */
export function waitingOn(decisions: RequiredDecision[], initiativeKey: string): (epicKey: string) => string | null {
	const first = new Map<string, string>();
	for (const d of blocking(decisions)) if (!first.has(d.on)) first.set(d.on, d.title);
	return (epicKey) => first.get(epicKey) ?? first.get(initiativeKey) ?? null;
}

/** The blocking decisions on the initiative itself or one of its epics, each named by the item it holds up. Decisions
 *  on stories are left to the epic page and the epic cards' counts. */
export function stripDecisions(decisions: RequiredDecision[], initiative: Pick<Initiative, 'key' | 'title'>, epics: Pick<Epic, 'key' | 'title'>[]): { onTitle: string }[] {
	const titles = new Map<string, string>([[initiative.key, initiative.title], ...epics.map((e) => [e.key, e.title] as [string, string])]);
	return blocking(decisions).flatMap((d) => {
		const onTitle = titles.get(d.on);
		return onTitle === undefined ? [] : [{ onTitle }];
	});
}
