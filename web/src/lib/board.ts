// Pure helpers for the delivery board: columns, reason labels, filters and rank placement.
import type { PromisedFilter, Readiness, Reason, SimilarStory, Story, StoryWithReadiness } from '#lib/api/types.ts';
import { promisedMatches } from '#lib/promise.ts';

export type Column = Story['delivery'];
export type Kind = Story['kind'];
export type Resolution = NonNullable<Story['resolution']>;

export const COLUMNS = ['todo', 'in_progress', 'in_review', 'testing', 'done'] as const satisfies readonly Column[];
// Fails `npm run check` if the server adds a delivery state that COLUMNS does not list.
const everyColumnListed: Exclude<Column, (typeof COLUMNS)[number]> extends never ? true : never = true;
void everyColumnListed;

export const COLUMN_LABEL: Record<Column, string> = {
	todo: 'todo',
	in_progress: 'in progress',
	in_review: 'in review',
	testing: 'testing',
	done: 'done'
};

/** The board's column headings: todo reads "up next" there only. COLUMN_LABEL keeps naming the delivery state (the
 *  drawer, the override prompt, the card's "Move to" items), kb/ADR-0002 R6. */
export const BOARD_HEADING: Record<Column, string> = { ...COLUMN_LABEL, todo: 'up next' };

/** Where a similar story stands: its delivery state, and for a finished one how it ended (shown in the Report dialog and in
 *  triage). A resolution on a story that is not done is stale and not shown. */
export const hitState = (h: Pick<SimilarStory, 'delivery' | 'resolution'>): string =>
	h.delivery === 'done' && h.resolution ? `done, ${h.resolution}` : COLUMN_LABEL[h.delivery];

/** The key of the full-column story right after `part`'s last story, or null (the part ends the column, or is empty).
 *  A move to the end of an Up next part ranks before it, so the card stays inside its part instead of landing after every
 *  story of the other part. */
export function afterPart(full: StoryWithReadiness[], part: StoryWithReadiness[]): string | null {
	const last = part.at(-1);
	if (!last) return null;
	const i = full.findIndex((s) => s.story.key === last.story.key);
	return i === -1 ? null : (full[i + 1]?.story.key ?? null);
}

/** The column heading: the done column says how far back it reaches (`GET /stories?done=recent`). Undefined and 0
 *  (a server from before the field existed) mean unknown, so the heading stays plain. */
export function columnHeading(col: Column, doneWindowDays: number | undefined): string {
	if (col !== 'done' || !doneWindowDays) return BOARD_HEADING[col];
	return `${BOARD_HEADING.done} · last ${doneWindowDays} ${doneWindowDays === 1 ? 'day' : 'days'}`;
}

/** Up next: the todo column, ready stories first; both parts keep the column's rank order. Readiness is the
 *  server's answer, never recomputed (kb/ADR-0027 R1). */
export function splitUpNext(todo: StoryWithReadiness[]): {
	ready: StoryWithReadiness[];
	notReady: StoryWithReadiness[];
} {
	return { ready: todo.filter((s) => s.readiness.ready), notReady: todo.filter((s) => !s.readiness.ready) };
}

// A Record over the generated union, so a new kind on the server fails the type check until it is listed here.
const KIND_SET: Record<Kind, true> = { story: true, bug: true, chore: true };
export const KINDS = Object.keys(KIND_SET) as Kind[];

export function isKind(s: string | null | undefined): s is Kind {
	return s != null && Object.hasOwn(KIND_SET, s);
}

/** A Record over `Reason['reason']` whose entry for each discriminant takes that variant, so the label can read its fields.
 *  Label coverage is checked by the compiler (`satisfies Labels`), not at runtime: the generated types are erased. */
type Labels = { [K in Reason['reason']]: (r: Extract<Reason, { reason: K }>) => string };

/** The short label a card shows. Typed over the generated union, so a reason added on the server fails `npm run check`. */
export const SHORT = {
	untriaged: () => 'untriaged',
	no_criteria: () => 'no criteria',
	ancestor_discovery_incomplete: (r) => `discovery open: ${r.item}`,
	decision_no_record: () => 'decision: no record',
	decision_unchecked: () => 'decision unchecked',
	decision_proposed_pr_open: () => 'decision: proposed (PR linked)',
	decision_record_not_found: () => 'decision: record not found',
	decision_repo_not_configured: () => 'decision: record in an unchecked repository',
	decision_not_accepted: (r) =>
		r.superseded_by ? `decision: superseded by ${r.superseded_by}, re-point` : `decision: ${r.status}`,
	blocked_by: (r) => `blocked by ${r.story} (${COLUMN_LABEL[r.delivery]})`,
	resolution_required: () => 'resolution required',
	completed_needs_active_origin: () => 'completed needs active work',
	override_needs_reason: () => 'override needs a reason',
	automation_cannot_override: () => 'automation cannot override',
	sprint_active: () => 'sprint active',
	verdict_missing: () => 'verdict missing',
	sprint_not_active: () => 'sprint not active',
	sprint_not_draft: () => 'sprint not draft',
	sprint_closed: () => 'sprint closed',
	interactive_only: () => 'interactive only'
} satisfies Labels;

/** What the blocker is doing and who can unblock it (kb/ADR-0027). */
function blockerClause(delivery: Column, owner?: string | null): string {
	if (delivery === 'todo') return owner ? `todo, owned by ${owner}` : 'todo, not yet started by anyone';
	const state = COLUMN_LABEL[delivery];
	return owner
		? `${state}, owned by ${owner}; it unblocks when merged, or ask its owner`
		: `${state}, no owner; it unblocks when merged`;
}

/** The full sentence: the badge's title attribute, the override prompt and refusal notices. */
export const FULL = {
	untriaged: () => 'untriaged: no epic yet; triage places it',
	no_criteria: () => 'no acceptance criteria',
	ancestor_discovery_incomplete: (r) => `discovery incomplete on ${r.item}`,
	decision_no_record: (r) => `decision "${r.title}" has no record linked`,
	decision_unchecked: (r) => `decision "${r.title}" has not been checked`,
	decision_proposed_pr_open: (r) => `decision "${r.title}" is proposed; its PR is linked`,
	decision_record_not_found: (r) => `decision "${r.title}": record not found on the default branch`,
	decision_repo_not_configured: (r) => `decision "${r.title}": record in an unchecked repository`,
	decision_not_accepted: (r) =>
		r.superseded_by
			? `decision "${r.title}": superseded by ${r.superseded_by}, re-point`
			: `decision "${r.title}" is ${r.status}`,
	blocked_by: (r) => `blocked by ${r.story}: ${blockerClause(r.delivery, r.owner)}`,
	resolution_required: () => 'a resolution is required to move to done',
	completed_needs_active_origin: () => 'completed needs the story to have been in active work',
	override_needs_reason: () => 'an override needs a reason',
	automation_cannot_override: () => 'automation tokens cannot override',
	sprint_active: () => 'a sprint is already active; close it first',
	verdict_missing: (r) => `every goal needs a verdict: ${r.goals.join(', ')}`,
	sprint_not_active: () => 'the sprint is not active',
	sprint_not_draft: () => "only a draft sprint's goals can be removed",
	sprint_closed: () => 'the sprint is closed',
	interactive_only: () => 'only an interactive user may do this'
} satisfies Labels;

function render(labels: Labels, r: Reason): string {
	// The map is keyed by the discriminant, so the entry for r.reason takes exactly r's variant.
	return (labels[r.reason] as (r: Reason) => string)(r);
}

export const shortLabel = (r: Reason): string => render(SHORT, r);
export const fullLabel = (r: Reason): string => render(FULL, r);

/** "ready", or the first reason rendered short. */
export function shortReason(r: Readiness): string {
	return r.ready || r.reasons.length === 0 ? 'ready' : shortLabel(r.reasons[0]);
}

export type StoryStatus = { word: string; tone: 'ready' | 'warn' | 'muted' };

/** A story's status word and dot tone, as board cards and the epic page's rows show it: a done story its resolution,
 *  else the server's readiness rendered short (kb/ADR-0027 R1). */
export function storyStatus(item: StoryWithReadiness): StoryStatus {
	const s = item.story;
	if (s.delivery === 'done') return { word: s.resolution ?? 'done', tone: 'muted' };
	return { word: shortReason(item.readiness), tone: item.readiness.ready ? 'ready' : 'warn' };
}

/** Todo by rank, then key; every other column by key. Throws on a delivery state outside COLUMNS. */
export function columnise(stories: StoryWithReadiness[]): Record<Column, StoryWithReadiness[]> {
	const out = Object.fromEntries(COLUMNS.map((c) => [c, [] as StoryWithReadiness[]])) as Record<
		Column,
		StoryWithReadiness[]
	>;
	for (const s of stories) {
		const col = Object.hasOwn(out, s.story.delivery) ? out[s.story.delivery] : undefined;
		if (!col) throw new Error(`unknown delivery state "${String(s.story.delivery)}" on ${s.story.key}`);
		col.push(s);
	}
	for (const c of COLUMNS) {
		out[c].sort((a, b) =>
			c === 'todo' && a.story.rank !== b.story.rank ? a.story.rank - b.story.rank : byKey(a.story.key, b.story.key)
		);
	}
	return out;
}

/** ABC-9 before ABC-10: shorter keys first, then lexically (the server's order). */
function byKey(a: string, b: string): number {
	return a.length - b.length || (a < b ? -1 : a > b ? 1 : 0);
}

/** `goal`: a goal's id keeps the stories that serve it, `'none'` the ones that serve no goal (upkeep); null or absent
 *  filters nothing. */
export type Filters = {
	mine?: string;
	epic?: string;
	kind?: Kind;
	untriaged?: boolean;
	goal?: number | 'none' | null;
	/** The server's promise state and period, read through `promisedMatches`; only unfinished stories match. */
	promised?: PromisedFilter | null;
};

/** Untriaged (kb/ADR-0027 R4: an item of any kind without an epic that is not done): the server's readiness reason,
 *  never re-derived from kind and epic on the client (kb/ADR-0027 R1), and not a finished item. */
export function isUntriaged(s: StoryWithReadiness): boolean {
	return s.story.delivery !== 'done' && s.readiness.reasons.some((r) => r.reason === 'untriaged');
}

/** `mine` is the caller's email; `untriaged` keeps the stories the server calls untriaged (`isUntriaged`); `goal` reads
 *  the goals the server says each story serves (`story.goals`, kb/ADR-0014 R3), never re-derived from links. */
export function applyFilters(stories: StoryWithReadiness[], f: Filters): StoryWithReadiness[] {
	return stories.filter((row) => {
		const s = row.story;
		if (f.mine && s.owner !== f.mine) return false;
		if (f.epic && s.epic !== f.epic) return false;
		if (f.kind && s.kind !== f.kind) return false;
		if (f.untriaged && !isUntriaged(row)) return false;
		if (f.goal === 'none' && s.goals.length > 0) return false;
		if (typeof f.goal === 'number' && !s.goals.some((g) => g.id === f.goal)) return false;
		if (f.promised != null && !(s.promise != null && promisedMatches(f.promised, s.promise))) return false;
		return true;
	});
}

/** `column` is the complete todo column in rank order (hidden cards included). Returns the keys in the order the server
 *  will hold after `POST /stories/{movedKey}/rank {before: beforeKey}` (before `beforeKey`, or at the end when null), or
 *  null when the placement changes nothing, so the board skips the request. The server does the renumbering. */
export function orderAfterDrop(
	column: StoryWithReadiness[],
	movedKey: string,
	beforeKey: string | null
): string[] | null {
	const keys = column.map((s) => s.story.key);
	if (!keys.includes(movedKey) || beforeKey === movedKey) return null;
	const rest = keys.filter((k) => k !== movedKey);
	const at = beforeKey === null ? -1 : rest.indexOf(beforeKey);
	rest.splice(at === -1 ? rest.length : at, 0, movedKey);
	return rest.every((k, i) => k === keys[i]) ? null : rest;
}

/** The resolutions a move into done may offer from `from` (kb/ADR-0002 R4: completed only from an active state, dropped
 *  from any). From done it is a re-resolution: a completed story may become dropped, a dropped one cannot become completed
 *  (it was never active since), so a dropped story offers nothing. */
export function resolutionsFor(from: Column, current: Resolution | null): Resolution[] {
	switch (from) {
		case 'todo':
			return ['dropped'];
		case 'done':
			return current === 'completed' ? ['dropped'] : [];
		case 'in_progress':
		case 'in_review':
		case 'testing':
			return ['completed', 'dropped'];
	}
}

/** Whether a drag from `from` may drop on `to`: another column, a reorder within todo, or a re-resolution within done. */
export function accepts(from: Column, to: Column, current: Resolution | null): boolean {
	if (from !== to) return true;
	if (to === 'todo') return true;
	return to === 'done' && resolutionsFor('done', current).length > 0;
}

/** The "move up / move down" target for `key` within the cards the board shows (`shown`, filters applied): up places it
 *  before the previous shown card, down before the shown card after the next one (or at the end). Returns null at the edge
 *  of the shown list. The server places the card against the FULL todo column, so hidden cards keep their place. */
export function neighbourBefore(
	shown: StoryWithReadiness[],
	key: string,
	direction: 'up' | 'down'
): { before: string | null } | null {
	const i = shown.findIndex((s) => s.story.key === key);
	if (i === -1) return null;
	if (direction === 'up') return i === 0 ? null : { before: shown[i - 1].story.key };
	if (i === shown.length - 1) return null;
	return { before: shown[i + 2]?.story.key ?? null };
}

/** Where a drop at `pointerY` lands among the shown cards' boxes (in display order): before the first card whose vertical
 *  midpoint is below the pointer, or at the end (null). A drop in a gap or on the header lands where it visually is. */
export function dropBefore(pointerY: number, boxes: { key: string; top: number; height: number }[]): string | null {
	return boxes.find((b) => b.top + b.height / 2 > pointerY)?.key ?? null;
}
