// web/src/lib/backlog.ts
// The backlog's "by epic" view: epic-less stories (all untriaged, one rank run) first, then every epic under its initiative in planning
// order, then epics with no initiative. Readiness is the server's (kb/ADR-0018 R1).
import type { Epic, Initiative, StoryWithReadiness } from '#lib/api/types.ts';
import { isUntriaged } from '#lib/board.ts';
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
