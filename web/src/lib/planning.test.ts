import { describe, expect, it } from 'vitest';
import { decision, epic as epicDto, initiative } from '#lib/test/fixtures.ts';
import { bucketise, groupByInitiative, groupEpics, horizonLayout, openDecisionsByItem, OTHER } from './planning.ts';

const ini = (key: string, horizon: string, title = key, target_month: string | null = null) =>
	initiative({ key, title, horizon, target_month });

const dec = (id: number, on: string, created_at: string, closed = false) =>
	decision({
		id,
		on,
		created_at,
		closed: closed ? { at: '2026-10-20T00:00:00Z', by: 'a@example.com', reason: 'decided elsewhere' } : null
	});

describe('bucketise', () => {
	it('puts every configured bucket in, in order, even when empty', () => {
		const b = bucketise(['Now', 'Next', 'Later'], [ini('R-1', 'Next')]);
		expect([...b.keys()]).toEqual(['Now', 'Next', 'Later']);
		expect(b.get('Now')).toEqual([]);
		expect(b.get('Next')!.map((i) => i.key)).toEqual(['R-1']);
	});

	it('keeps an initiative whose horizon is no longer configured under Other', () => {
		const b = bucketise(['Now', 'Next'], [ini('R-1', 'Now'), ini('R-2', 'Someday')]);
		expect([...b.keys()]).toEqual(['Now', 'Next', OTHER]);
		expect(b.get(OTHER)!.map((i) => i.key)).toEqual(['R-2']);
	});

	it('adds no Other column when every horizon is configured', () => {
		expect(bucketise(['Now'], [ini('R-1', 'Now')]).has(OTHER)).toBe(false);
	});

	it('sorts each bucket by target month, then title, months before none', () => {
		const b = bucketise(
			['Now'],
			[
				ini('R-1', 'Now', 'b', null),
				ini('R-2', 'Now', 'z', '2026-12'),
				ini('R-3', 'Now', 'b', '2026-11'),
				ini('R-4', 'Now', 'a', '2026-11'),
				ini('R-5', 'Now', 'a', null)
			]
		);
		expect(b.get('Now')!.map((i) => i.key)).toEqual(['R-4', 'R-3', 'R-2', 'R-5', 'R-1']);
	});
});

describe('openDecisionsByItem', () => {
	it('does not count a closed decision', () => {
		const m = openDecisionsByItem([
			dec(1, 'R-1', '2026-10-01T00:00:00Z', true),
			dec(2, 'R-1', '2026-10-10T00:00:00Z'),
			dec(3, 'R-2', '2026-10-05T00:00:00Z', true)
		]);
		expect(m.get('R-1')?.count).toBe(1);
		expect(m.get('R-1')?.oldest).toBe('2026-10-10T00:00:00Z');
		expect(m.has('R-2')).toBe(false);
	});

	it('takes the earliest created_at as the oldest, not the first in input order', () => {
		const m = openDecisionsByItem([
			dec(1, 'R-1', '2026-10-20T00:00:00Z'),
			dec(2, 'R-1', '2026-09-01T00:00:00Z'),
			dec(3, 'R-1', '2026-10-01T00:00:00Z')
		]);
		expect(m.get('R-1')).toMatchObject({ count: 3, oldest: '2026-09-01T00:00:00Z' });
		expect(m.get('R-1')!.open.map((d) => d.id)).toEqual([2, 3, 1]);
	});
});

const epic = (key: string, initiative: string | null) => epicDto({ key, title: key, initiative });

describe('groupEpics', () => {
	const inis = [ini('R-1', 'Later'), ini('R-2', 'Now', 'B', '2026-12'), ini('R-3', 'Now', 'A', '2026-11'), ini('R-4', 'Now')];
	const epics = [
		epic('R-12', 'R-2'), epic('R-11', null), epic('R-9', 'R-2'), epic('R-13', 'R-99'), epic('R-10', 'R-2'),
		epic('R-15', 'R-100'), epic('R-14', 'R-1')
	];
	const groups = groupEpics(['Now', 'Next', 'Later'], inis, epics);
	it('lists initiatives in the planning order, then unlisted parents as first seen, then No initiative', () => {
		expect(groups.map((g) => g.key)).toEqual(['R-3', 'R-2', 'R-4', 'R-1', 'R-99', 'R-100', null]);
		expect(groups.at(-3)).toMatchObject({ initiative: null, title: 'R-99' });
		expect(groups.at(-1)).toMatchObject({ initiative: null, title: 'No initiative' });
	});
	it('keeps the server order inside a group and empty initiatives with no epics', () => {
		expect(groups[1].epics.map((e) => e.key)).toEqual(['R-12', 'R-9', 'R-10']);
		expect(groups[2].epics).toEqual([]);
	});
	it('leaves out No initiative when every epic has one', () => {
		expect(groupEpics(['Now'], [ini('R-1', 'Now')], [epic('R-2', 'R-1')]).map((g) => g.key)).toEqual(['R-1']);
	});
});

describe('horizonLayout', () => {
	it('shows every configured horizon but the last as a column and collapses the last', () => {
		const b = bucketise(['Now', 'Next', 'Later'], []);
		expect(horizonLayout(['Now', 'Next', 'Later'], b)).toEqual({ columns: ['Now', 'Next'], collapsed: 'Later' });
	});

	it('keeps Other as a column after the configured ones when present', () => {
		const b = bucketise(['Now', 'Next', 'Later'], [ini('R-1', 'Someday')]);
		expect(horizonLayout(['Now', 'Next', 'Later'], b)).toEqual({ columns: ['Now', 'Next', OTHER], collapsed: 'Later' });
	});

	it('puts the expanded last horizon in configured order, before the synthetic Other', () => {
		const b = bucketise(['Now', 'Next', 'Later'], [ini('R-1', 'Someday')]);
		expect(horizonLayout(['Now', 'Next', 'Later'], b, true)).toEqual({
			columns: ['Now', 'Next', 'Later', OTHER],
			collapsed: 'Later'
		});
	});

	it('treats a configured horizon named Other as a horizon, not the fallback', () => {
		const conf = ['Now', OTHER, 'Later'];
		const b = bucketise(conf, [ini('R-1', OTHER)]);
		expect(horizonLayout(conf, b)).toEqual({ columns: ['Now', OTHER], collapsed: 'Later' });
		expect(horizonLayout(conf, b, true)).toEqual({ columns: ['Now', OTHER, 'Later'], collapsed: 'Later' });
	});

	it('collapses nothing when a single horizon is configured', () => {
		const b = bucketise(['Now'], []);
		expect(horizonLayout(['Now'], b)).toEqual({ columns: ['Now'], collapsed: null });
	});

	it('shows only Other when no horizon is configured', () => {
		expect(horizonLayout([], bucketise([], []))).toEqual({ columns: [], collapsed: null });
		expect(horizonLayout([], bucketise([], [ini('R-1', 'Now')]))).toEqual({ columns: [OTHER], collapsed: null });
	});
});

describe('groupByInitiative', () => {
	type G = { id: number; inis: string[] };
	const inis = [ini('R-1', 'Later'), ini('R-2', 'Now', 'B'), ini('R-3', 'Now', 'A')];
	const goals: G[] = [
		{ id: 1, inis: ['R-1', 'R-2'] },
		{ id: 2, inis: [] },
		{ id: 3, inis: ['R-99'] },
		{ id: 4, inis: ['R-3'] }
	];
	const groups = groupByInitiative(['Now', 'Next', 'Later'], inis, goals, (g) => g.inis[0] ?? null, 'Upkeep / no initiative');
	it('puts an item under its first initiative, groups in planning order, unlisted keys after, the none group last', () => {
		expect(groups.map((g) => [g.key, g.items.map((x) => x.id)])).toEqual([
			['R-3', [4]],
			['R-2', []],
			['R-1', [1]],
			['R-99', [3]],
			[null, [2]]
		]);
		expect(groups.at(-2)).toMatchObject({ title: 'R-99', initiative: null });
		expect(groups.at(-1)).toMatchObject({ title: 'Upkeep / no initiative', initiative: null });
	});
});
