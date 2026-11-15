// Pure helpers for the epic page: the decisions its status strip names.
// Readiness and blocking come from server data only (kb/ADR-0027 R1/R3).
import type { Epic, RequiredDecision, StoryWithReadiness } from '#lib/api/types.ts';
import { blocking } from '#lib/decisions.ts';

/** The blocking decisions on the epic itself, then those on its initiative (which hold up every epic under it), each
 *  named by the item it holds up. `underInitiative` is `GET /decisions?initiative=`: every decision under the initiative,
 *  so only those on the initiative itself are kept (the epic's own come from `onEpic`, other epics' are not this page's). */
export function epicStripDecisions(
	onEpic: RequiredDecision[],
	underInitiative: RequiredDecision[],
	epic: Pick<Epic, 'key' | 'title' | 'initiative'>,
	initiativeTitle: string | null
): { onTitle: string }[] {
	const own = blocking(onEpic.filter((d) => d.on === epic.key)).map(() => ({ onTitle: epic.title }));
	const ini = epic.initiative;
	if (!ini) return own;
	const above = blocking(underInitiative.filter((d) => d.on === ini)).map(() => ({ onTitle: initiativeTitle ?? ini }));
	return [...own, ...above];
}

/** Whether the page's two-minute refetch has work: an open decision with a record to check on the epic, one of its
 *  stories, or its initiative itself. `underInitiative` also holds sibling epics' decisions, which are not this page's.
 *  An epic under no initiative has no initiative list, so its stories' decisions are not known here. */
export function pollsChecks(
	onEpic: RequiredDecision[],
	underInitiative: RequiredDecision[],
	epic: Pick<Epic, 'key' | 'initiative'>,
	stories: StoryWithReadiness[]
): boolean {
	const mine = new Set([epic.key, ...stories.map((s) => s.story.key)]);
	if (epic.initiative) mine.add(epic.initiative);
	return [...onEpic, ...underInitiative].some((d) => mine.has(d.on) && !!d.record && !d.closed);
}
