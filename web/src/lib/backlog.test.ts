// web/src/lib/backlog.test.ts
import { describe, expect, it } from 'vitest';
import type { Reason, Story } from '#lib/api/types.ts';
import { epic as epicDto, initiative, withReadiness } from '#lib/test/fixtures.ts';
import { groupBacklog } from './backlog.ts';

const ini = (key: string, horizon: string, title = key) => initiative({ key, title, horizon });
const epic = (key: string, initiative: string | null) => epicDto({ key, title: `Epic ${key}`, initiative });
const row = (key: string, rank: number, opts: { epic?: string; kind?: Story['kind']; ready?: boolean; untriaged?: boolean } = {}) => {
	const reasons: Reason[] = opts.ready ? [] : opts.untriaged ? [{ reason: 'untriaged' }, { reason: 'no_criteria' }] : [{ reason: 'no_criteria' }];
	return withReadiness({ key, rank, title: key, kind: opts.kind ?? 'story', epic: opts.epic ?? null, delivery: 'todo' }, reasons);
};

describe('groupBacklog', () => {
	const buckets = ['Now', 'Next'];
	const initiatives = [ini('I-3', 'Next', 'Search'), ini('I-2', 'Next', 'Billing'), ini('I-1', 'Now', 'Onboarding')];
	const epics = [epic('E-1', 'I-1'), epic('E-2', 'I-1'), epic('E-3', null), epic('E-4', 'I-2')];
	const todo = [
		row('S-5', 5, { epic: 'E-1', ready: true }),
		row('S-1', 1, { kind: 'chore' }),
		row('S-2', 2, { epic: 'E-1' }),
		row('S-3', 3, { kind: 'story', untriaged: true }),
		row('S-4', 4, { epic: 'E-3', ready: true }),
		row('S-0', 0, { kind: 'chore' }),
		row('S-6', 6, { epic: 'E-9' })
	];
	const b = groupBacklog(buckets, initiatives, epics, todo);

	it('No epic first: untriaged stories before epic-less bugs and chores, rank order inside each', () => {
		expect(b.noEpic.map((r) => r.story.key)).toEqual(['S-3', 'S-0', 'S-1']);
		expect(b.untriaged).toBe(1);
	});
	it('untriaged is the server reason, not kind and epic re-derived', () => {
		const g = groupBacklog(buckets, initiatives, epics, [row('S-7', 7, { kind: 'chore' }), row('S-8', 8, { kind: 'story' })]);
		expect(g.untriaged).toBe(0);
		expect(g.noEpic.map((r) => r.story.key)).toEqual(['S-7', 'S-8']); // rank order: no untriaged run first
	});
	it('initiatives in planning order, every epic listed, an empty one says so', () => {
		expect(b.sections.map((s) => s.title)).toEqual(['Onboarding', 'Billing', 'Search', 'No initiative']);
		const i1 = b.sections[0].epics;
		expect(i1.map((e) => e.epic.key)).toEqual(['E-1', 'E-2']);
		expect(i1[0].rows.map((r) => r.story.key)).toEqual(['S-2', 'S-5']); // rank order, not input order
		expect(i1[0].summary).toBe('1 of 2 ready');
		expect(i1[0].tone).toBe('warn');
		expect(i1[1].tone).toBe('muted');
		expect(i1[1].summary).toBe('No stories waiting');
		expect(b.sections[1].epics[0].summary).toBe('No stories waiting'); // E-4, no todo stories
		expect(b.sections[1].epics[0].tone).toBe('muted');
		expect(b.sections[2].epics).toEqual([]); // an initiative without epics is kept
		expect(b.sections[2].initiative?.key).toBe('I-3');
	});
	it('stories whose epic is unknown are returned as orphans in rank order', () => {
		expect(b.orphans.map((r) => r.story.key)).toEqual(['S-6']);
	});
	it('epics without an initiative go under No initiative', () => {
		expect(b.sections[3].epics.map((e) => e.epic.key)).toEqual(['E-3']);
		expect(b.sections[3].epics[0].tone).toBe('ready');
	});
});
