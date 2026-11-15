// Pure helpers for the delivery board: columns, reason labels, filters and rank placement.
import type { PromisedFilter, Readiness, Reason, SimilarStory, Story, StoryWithReadiness } from '#lib/api/types.ts';
import { sameEmail, short } from '#lib/format.ts';
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

/** `owner`: an email keeps that person's stories, `'none'` the unowned ones. `goal`: a goal's id keeps the stories that
 *  serve it, `'any'` the ones that serve some goal of the active sprint, `'none'` the ones that serve no goal (upkeep).
 *  Absent or null filters nothing. */
export type Filters = {
	owner?: string | 'none';
	epic?: string;
	kind?: Kind;
	untriaged?: boolean;
	goal?: number | 'any' | 'none' | null;
	/** The server's promise state and period, read through `promisedMatches`; only unfinished stories match. */
	promised?: PromisedFilter | null;
};

/** Untriaged (kb/ADR-0027 R4: an item of any kind without an epic that is not done): the server's readiness reason,
 *  never re-derived from kind and epic on the client (kb/ADR-0027 R1), and not a finished item. */
export function isUntriaged(s: StoryWithReadiness): boolean {
	return s.story.delivery !== 'done' && s.readiness.reasons.some((r) => r.reason === 'untriaged');
}

/** `untriaged` keeps the stories the server calls untriaged (`isUntriaged`); `goal` reads the goals the server says
 *  each story serves (`story.goals`, the active sprint's only, kb/ADR-0014 R6), never re-derived from links. */
export function applyFilters(stories: StoryWithReadiness[], f: Filters): StoryWithReadiness[] {
	return stories.filter((row) => {
		const s = row.story;
		if (f.owner === 'none' && s.owner != null) return false;
		if (f.owner != null && f.owner !== 'none' && !(s.owner != null && sameEmail(s.owner, f.owner))) return false;
		if (f.epic && s.epic !== f.epic) return false;
		if (f.kind && s.kind !== f.kind) return false;
		if (f.untriaged && !isUntriaged(row)) return false;
		if (f.goal === 'none' && s.goals.length > 0) return false;
		if (f.goal === 'any' && s.goals.length === 0) return false;
		if (typeof f.goal === 'number' && !s.goals.some((g) => g.id === f.goal)) return false;
		if (f.promised != null && !(s.promise != null && promisedMatches(f.promised, s.promise))) return false;
		return true;
	});
}

/** The board's owner filter from its URL: `owner=none` (trimmed, in any case) is the unowned stories, `owner=me` (the
 *  same) the signed-in person's, as `mine=1` is (nobody's while signed out), any other `owner` that person's, else
 *  `mine=1`. `owner` wins when both are set. `none` and `me` are the board's own URL values, reserved because a real
 *  address contains `@`: the server's `owner` parameter takes an email, and the board sends neither. */
export function ownerFilter(query: Pick<URLSearchParams, 'get'>, me: string | null | undefined): string | 'none' | undefined {
	const owner = query.get('owner')?.trim();
	if (owner) {
		const word = owner.toLowerCase();
		if (word === 'none') return 'none';
		if (word === 'me') return me || undefined;
		return owner;
	}
	return query.get('mine') === '1' && me ? me : undefined;
}

export type OwnerOption = { email: string; label: string };

/** The Owner select's people: who owns a card among `stories` (every loaded card, before any filter), without the
 *  signed-in person (the select's "Me"), plus `selected` when it names someone with no card, so an applied filter
 *  always has its option. Listing users is admin-only, which is why the board is the source. The label is the email's
 *  local part, or the whole email when two of the people the select stands for (the signed-in person included) share
 *  one. Sorted by label, by code unit, so the order is the same in every locale. */
export function boardOwners(
	stories: StoryWithReadiness[],
	me: string | null | undefined,
	selected: string | 'none' | undefined
): OwnerOption[] {
	const id = (email: string) => email.trim().toLowerCase();
	const emails = new Map<string, string>();
	const add = (email: string | null | undefined) => {
		if (!email || (me && sameEmail(email, me)) || emails.has(id(email))) return;
		emails.set(id(email), email);
	};
	for (const row of stories) add(row.story.owner);
	if (selected !== 'none') add(selected);

	const locals = new Map<string, number>();
	for (const email of [...emails.values(), ...(me ? [me] : [])]) {
		const local = short(email).toLowerCase();
		locals.set(local, (locals.get(local) ?? 0) + 1);
	}
	return [...emails.values()]
		.map((email) => ({ email, label: (locals.get(short(email).toLowerCase()) ?? 0) > 1 ? email : short(email) }))
		.sort((a, b) => {
			const [x, y] = [a.label.toLowerCase(), b.label.toLowerCase()];
			return x < y ? -1 : x > y ? 1 : 0;
		});
}

/** The Owner select's value for a filter: `''` (no filter), `'me'`, `'none'`, or the email exactly as its option
 *  holds it. A select shows the option whose value is identical to its own, so the URL's spelling of an email (another
 *  case, padded) would show none. `options` is expected to come from `boardOwners(…, owner)`, which always includes the
 *  selected owner; if it does not, the URL's spelling is returned and the select shows no option. */
export function ownerSelectValue(
	owner: string | 'none' | undefined,
	me: string | null | undefined,
	options: readonly OwnerOption[]
): string {
	if (owner == null) return '';
	if (owner === 'none') return 'none';
	if (me && sameEmail(owner, me)) return 'me';
	return options.find((o) => sameEmail(o.email, owner))?.email ?? owner;
}

/** The URL parameters `Clear filters` removes. `notready` is a disclosure's state, not a filter, and stays. */
export const BOARD_FILTER_PARAMS = ['mine', 'owner', 'goal', 'epic', 'kind', 'promised', 'untriaged'] as const;

/** Clear filters (n): the filters the board applies, each counted once (a `?goal=` it cannot apply is not one). */
export function boardFilterCount(f: Filters): number {
	return [f.owner != null, f.goal != null, !!f.epic, !!f.kind, f.promised != null, !!f.untriaged].filter(Boolean).length;
}

/** Clear filters: a copy of the URL without the board's filter parameters. */
export function withoutBoardFilters(url: URL): URL {
	const next = new URL(url.href);
	for (const name of BOARD_FILTER_PARAMS) next.searchParams.delete(name);
	return next;
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
