// web/src/lib/backlog.ts
// The backlog's "by epic" view: epic-less stories (all untriaged, one rank run) first, then every epic under its initiative in planning
// order, then epics with no initiative. Readiness is the server's (kb/ADR-0018 R1).
import type { Epic, Initiative, StoryQuery, StoryWithReadiness } from '#lib/api/types.ts';
import { isKind, isUntriaged } from '#lib/board.ts';
import { groupEpics } from '#lib/planning.ts';

export type Tone = 'ready' | 'warn' | 'muted';
export type EpicRows = { epic: Epic; rows: StoryWithReadiness[]; summary: string; tone: Tone };
export type Backlog = {
	noEpic: StoryWithReadiness[];
	orphans: StoryWithReadiness[];
	untriaged: number;
	sections: { key: string | null; title: string; initiative: Initiative | null; epics: EpicRows[] }[];
};

const byRank = (a: StoryWithReadiness, b: StoryWithReadiness) => a.story.rank - b.story.rank;

export function groupBacklog(buckets: string[], initiatives: Initiative[], epics: Epic[], todo: StoryWithReadiness[]): Backlog {
	const ranked = [...todo].sort(byRank);
	const noEpic = ranked.filter((s) => !s.story.epic);
	const byEpic = new Map<string, StoryWithReadiness[]>();
	for (const s of ranked) {
		if (!s.story.epic) continue;
		const list = byEpic.get(s.story.epic);
		if (list) list.push(s);
		else byEpic.set(s.story.epic, [s]);
	}
	const known = new Set(epics.map((e) => e.key));
	const orphans = ranked.filter((s) => s.story.epic && !known.has(s.story.epic));
	const sections = groupEpics(buckets, initiatives, epics).map((g) => ({
		key: g.key,
		title: g.title,
		initiative: g.initiative,
		epics: g.epics.map((epic) => {
			const rows = byEpic.get(epic.key) ?? [];
			const ready = rows.filter((s) => s.readiness.ready).length;
			const tone: Tone = rows.length === 0 ? 'muted' : ready === rows.length ? 'ready' : 'warn';
			return { epic, rows, summary: rows.length ? `${ready} of ${rows.length} ready` : 'No stories waiting', tone };
		})
	}));
	return { noEpic, orphans, untriaged: noEpic.filter(isUntriaged).length, sections };
}

/** Todo (the rankable list), Open (no done stories) or All. */
export type BacklogState = 'todo' | 'open' | 'all';
/** By epic, Ranked (flat, server order) or Relevance (flat, the server's relevance order; needs text). */
export type BacklogView = 'epic' | 'rank' | 'relevance';

/** The backlog's text filter, trimmed as the server reads it; null when there is none. */
export function backlogText(url: URL): string | null {
	return url.searchParams.get('q')?.trim() || null;
}

export function backlogState(url: URL): BacklogState {
	const s = url.searchParams.get('state');
	return s === 'open' || s === 'all' ? s : 'todo';
}

/** `view`, else the legacy `group=rank`; Relevance without text shows as Ranked. */
export function backlogView(url: URL): BacklogView {
	const v = url.searchParams.get('view') ?? (url.searchParams.get('group') === 'rank' ? 'rank' : null);
	if (v === 'relevance') return backlogText(url) ? 'relevance' : 'rank';
	return v === 'rank' ? 'rank' : 'epic';
}

/** The backlog URL's filters (not its state, view or text), by URL param, as `GET /stories` reads them. `me` is the
 *  signed-in email: Mine is dropped while signed out. */
export function backlogFilters(url: URL, me: string | null | undefined): Partial<StoryQuery> {
	const p = url.searchParams;
	const f: Partial<StoryQuery> = {};
	if (p.get('mine') === '1' && me) f.owner = me;
	const kind = p.get('kind');
	if (isKind(kind)) f.kind = kind;
	for (const name of ['epic', 'initiative', 'label'] as const) {
		const v = p.get(name);
		if (v) f[name] = v;
	}
	for (const name of ['ready', 'untriaged', 'urgent'] as const) if (p.get(name) === '1') f[name] = true;
	return f;
}

/** The URL params Filters (n) counts and Clear filters removes. */
export const FILTER_PARAMS = ['mine', 'kind', 'epic', 'initiative', 'label', 'ready', 'untriaged', 'urgent'] as const;

/** The backlog URL as a `GET /stories` query, or null for the plain backlog (Todo, no filter, no text): its rows are the
 *  unfiltered todo list the page fetches anyway. Built in a fixed key order, so equal queries serialise equally. */
export function toStoryQuery(url: URL, me: string | null | undefined): StoryQuery | null {
	const q = backlogText(url);
	const state = backlogState(url);
	const filters = backlogFilters(url, me);
	if (!q && state === 'todo' && Object.keys(filters).length === 0) return null;
	const out: StoryQuery = {};
	if (q) out.q = q;
	if (state === 'todo') out.delivery = 'todo';
	else out.done = state === 'open' ? 'none' : 'all';
	if (backlogView(url) === 'relevance') out.sort = 'relevance';
	return { ...out, ...filters };
}
