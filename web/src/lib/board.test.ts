import { describe, expect, it } from 'vitest';
import type { Reason, Story, StoryPromise, StoryWithReadiness } from '#lib/api/types.ts';
import {
	accepts,
	afterPart,
	applyFilters,
	columnHeading,
	columnise,
	COLUMNS,
	dropBefore,
	fullLabel,
	neighbourBefore,
	orderAfterDrop,
	resolutionsFor,
	shortLabel,
	shortReason,
	splitUpNext,
	storyStatus
} from '#lib/board.ts';
import { withReadiness } from '#lib/test/fixtures.ts';

function sw(key: string, over: Partial<Story> = {}, reasons: Reason[] = []): StoryWithReadiness {
	return withReadiness({ key, epic: 'ABC-1', ...over }, reasons);
}

describe('reason labels', () => {
	it('renders untriaged, a proposed decision with its title and a blocker with its key', () => {
		expect(shortLabel({ reason: 'untriaged' })).toBe('untriaged');
		expect(fullLabel({ reason: 'untriaged' })).toBe('untriaged: no epic yet; triage places it');

		const proposed: Reason = { reason: 'decision_proposed_pr_open', decision_id: 7, title: 'Use Postgres' };
		expect(shortLabel(proposed)).toBe('decision: proposed (PR linked)');
		expect(fullLabel(proposed)).toBe('decision "Use Postgres" is proposed; its PR is linked');

		expect(shortLabel({ reason: 'blocked_by', story: 'ABC-12', delivery: 'in_review', owner: 'dan@example.com' })).toBe(
			'blocked by ABC-12 (in review)'
		);
	});

	it('says who can unblock a blocker', () => {
		const b = (delivery: 'todo' | 'in_progress' | 'in_review', owner: string | null) =>
			fullLabel({ reason: 'blocked_by', story: 'ABC-4', delivery, owner });
		expect(b('todo', null)).toBe('blocked by ABC-4: todo, not yet started by anyone');
		expect(b('todo', 'dan@example.com')).toBe('blocked by ABC-4: todo, owned by dan@example.com');
		expect(b('in_review', 'dan@example.com')).toBe(
			'blocked by ABC-4: in review, owned by dan@example.com; it unblocks when merged, or ask its owner'
		);
		expect(b('in_progress', null)).toBe('blocked by ABC-4: in progress, no owner; it unblocks when merged');
	});

	it('shortReason says ready, or renders only the first reason', () => {
		expect(shortReason({ ready: true, reasons: [] })).toBe('ready');
		expect(
			shortReason({
				ready: false,
				reasons: [{ reason: 'no_criteria' }, { reason: 'blocked_by', story: 'ABC-9', delivery: 'todo', owner: null }]
			})
		).toBe('no criteria');
	});

});

describe('columnise', () => {
	it('sorts todo by rank then key, other columns by key', () => {
		const cols = columnise([
			sw('ABC-10', { rank: 1 }),
			sw('ABC-9', { rank: 1 }),
			sw('ABC-2', { rank: 5 }),
			sw('ABC-3', { rank: 0 }),
			sw('ABC-12', { delivery: 'testing', rank: 0 }),
			sw('ABC-11', { delivery: 'testing', rank: 9 })
		]);
		expect(Object.keys(cols)).toEqual([...COLUMNS]);
		expect(cols.todo.map((s) => s.story.key)).toEqual(['ABC-3', 'ABC-9', 'ABC-10', 'ABC-2']);
		expect(cols.testing.map((s) => s.story.key)).toEqual(['ABC-11', 'ABC-12']);
		expect(cols.done).toEqual([]);
	});

	it('throws on a delivery state outside COLUMNS', () => {
		const drifted = sw('ABC-4', { delivery: 'blocked' as Story['delivery'] });
		expect(() => columnise([drifted])).toThrow(/blocked/);
	});
});

describe('applyFilters', () => {
	const all = [
		sw('ABC-2', { owner: 'ana@x.se', kind: 'bug' }),
		sw('ABC-3', { owner: 'bo@x.se', epic: 'ABC-7' }),
		sw('ABC-4', { epic: null }, [{ reason: 'untriaged' }]),
		sw('ABC-5', { epic: null, kind: 'chore' }, [{ reason: 'untriaged' }]),
		// The client must not re-derive it: no epic but no untriaged reason from the server, so not untriaged.
		sw('ABC-6', { epic: null }),
		// Done items leave the filter even if they carry the reason (kb/ADR-0018 R4: epic-less and not done).
		sw('ABC-8', { epic: null, delivery: 'done' }, [{ reason: 'untriaged' }])
	];
	const keys = (f: Parameters<typeof applyFilters>[1]) => applyFilters(all, f).map((s) => s.story.key);

	it('filters by owner, epic, kind and untriaged, and combines them', () => {
		expect(keys({})).toEqual(['ABC-2', 'ABC-3', 'ABC-4', 'ABC-5', 'ABC-6', 'ABC-8']);
		expect(keys({ mine: 'ana@x.se' })).toEqual(['ABC-2']);
		expect(keys({ epic: 'ABC-7' })).toEqual(['ABC-3']);
		expect(keys({ kind: 'bug' })).toEqual(['ABC-2']);
		// Only the server's untriaged reason counts, whatever the kind (the client never re-derives it from kind and epic).
		expect(keys({ untriaged: true })).toEqual(['ABC-4', 'ABC-5']);
		expect(keys({ mine: 'ana@x.se', kind: 'story' })).toEqual([]);
	});

	it('filters by the goals the server says a story serves: one goal, or none (upkeep)', () => {
		const ref = (id: number) => ({ id, outcome: `goal ${id}`, via: 'story' as const });
		const rows = [
			sw('ABC-2', { goals: [ref(1)] }),
			sw('ABC-3', { goals: [ref(2), ref(1)], delivery: 'in_progress' }),
			sw('ABC-4', { goals: [ref(2)] }),
			sw('ABC-5')
		];
		const by = (f: Parameters<typeof applyFilters>[1]) => applyFilters(rows, f).map((s) => s.story.key);
		expect(by({ goal: 1 })).toEqual(['ABC-2', 'ABC-3']);
		expect(by({ goal: 2 })).toEqual(['ABC-3', 'ABC-4']);
		expect(by({ goal: 'none' })).toEqual(['ABC-5']);
		expect(by({ goal: null })).toEqual(['ABC-2', 'ABC-3', 'ABC-4', 'ABC-5']);
		expect(by({ goal: 1, kind: 'bug' })).toEqual([]);
	});

	it('promised: keeps the stories the server classifies, as promisedMatches reads state and due; done stories never match', () => {
		const pr = (state: StoryPromise['state'], due: StoryPromise['due'], days: number | null = null): StoryPromise => ({
			own: { on: '2026-11-20', to: null, by: 'dana@x.se', at: '2026-11-01T09:00:00Z' },
			epic: null,
			on: '2026-11-20',
			holder: 'own',
			state,
			due,
			days
		});
		const rows = [
			sw('ABC-2', { promise: pr('open', 'this_sprint') }),
			sw('ABC-3', { promise: pr('overdue', null, 3), delivery: 'in_progress' }),
			sw('ABC-4', { promise: pr('open', 'next_sprint') }),
			sw('ABC-5', { promise: pr('open', 'later') }),
			sw('ABC-6', { promise: pr('kept', null), delivery: 'done' }),
			sw('ABC-7', { promise: pr('late', null, 2), delivery: 'done' }),
			sw('ABC-8')
		];
		const by = (f: Parameters<typeof applyFilters>[1]) => applyFilters(rows, f).map((s) => s.story.key);
		expect(by({ promised: 'this_sprint' })).toEqual(['ABC-2', 'ABC-3']);
		expect(by({ promised: 'next_sprint' })).toEqual(['ABC-4']);
		expect(by({ promised: 'overdue' })).toEqual(['ABC-3']);
		expect(by({ promised: 'any' })).toEqual(['ABC-2', 'ABC-3', 'ABC-4', 'ABC-5']);
		expect(by({ promised: null })).toHaveLength(7);
		// The done column is empty while the filter is set.
		expect(columnise(applyFilters(rows, { promised: 'this_sprint' })).done).toEqual([]);
	});
});

describe('orderAfterDrop', () => {
	const todo = (...keys: string[]) => keys.map((k, i) => sw(k, { rank: i }));

	it('moves the last card before the first', () => {
		expect(orderAfterDrop(todo('A', 'B', 'C', 'D'), 'D', 'A')).toEqual(['D', 'A', 'B', 'C']);
	});

	it('moves a card to the end when there is no card to go before', () => {
		expect(orderAfterDrop(todo('A', 'B', 'C'), 'A', null)).toEqual(['B', 'C', 'A']);
	});

	it('keeps a hidden card between the visible neighbours in place', () => {
		// The board shows A, B, D (H is filtered out); dragging D onto B places it before B in the full list.
		expect(orderAfterDrop(todo('A', 'H', 'B', 'D'), 'D', 'B')).toEqual(['A', 'H', 'D', 'B']);
	});

	it('changes nothing when a card is dropped before itself, before its current successor, or is unknown', () => {
		expect(orderAfterDrop(todo('A', 'B', 'C'), 'B', 'B')).toBeNull();
		expect(orderAfterDrop(todo('A', 'B', 'C'), 'A', 'B')).toBeNull();
		expect(orderAfterDrop(todo('A', 'B', 'C'), 'C', null)).toBeNull();
		expect(orderAfterDrop(todo('A', 'B', 'C'), 'X', 'A')).toBeNull();
	});
});

describe('resolutionsFor', () => {
	it('offers completed only from an active column, dropped from todo, and re-resolves done only completed -> dropped', () => {
		expect(resolutionsFor('todo', null)).toEqual(['dropped']);
		for (const active of ['in_progress', 'in_review', 'testing'] as const) {
			expect(resolutionsFor(active, null)).toEqual(['completed', 'dropped']);
		}
		// The server refuses completed from done (CompletedNeedsActiveOrigin), and a same-resolution move is a no-op.
		expect(resolutionsFor('done', 'completed')).toEqual(['dropped']);
		expect(resolutionsFor('done', 'dropped')).toEqual([]);
	});

	it('accepts a drop on another column, within todo, and on done only when done can be re-resolved', () => {
		expect(accepts('todo', 'in_progress', null)).toBe(true);
		expect(accepts('todo', 'todo', null)).toBe(true);
		expect(accepts('in_review', 'in_review', null)).toBe(false);
		expect(accepts('done', 'done', 'completed')).toBe(true);
		expect(accepts('done', 'done', 'dropped')).toBe(false);
	});
});

describe('neighbourBefore', () => {
	const todo = (...keys: string[]) => keys.map((k, i) => sw(k, { rank: i }));
	// Full todo A, H, B, D with H filtered out: the board shows A, B, D.
	const full = todo('A', 'H', 'B', 'D');
	const shown = full.filter((s) => s.story.key !== 'H');
	const order = (moved: string, before: string | null) => orderAfterDrop(full, moved, before);

	it('moves up before the previous shown card, skipping a hidden card between them', () => {
		// Against the full list the previous card of B is the hidden H; the shown neighbour is A.
		expect(neighbourBefore(shown, 'B', 'up')).toEqual({ before: 'A' });
		expect(order('B', 'A')).toEqual(['B', 'A', 'H', 'D']);
		expect(neighbourBefore(shown, 'D', 'up')).toEqual({ before: 'B' });
		expect(order('D', 'B')).toEqual(['A', 'H', 'D', 'B']);
	});

	it('moves down past the next shown card, or to the end', () => {
		// A passes the hidden H and the shown B, landing before D.
		expect(neighbourBefore(shown, 'A', 'down')).toEqual({ before: 'D' });
		expect(order('A', 'D')).toEqual(['H', 'B', 'A', 'D']);
		expect(neighbourBefore(shown, 'B', 'down')).toEqual({ before: null });
		expect(order('B', null)).toEqual(['A', 'H', 'D', 'B']);
	});

	it('returns null at either edge of the shown list and for a key it does not show', () => {
		expect(neighbourBefore(shown, 'A', 'up')).toBeNull();
		expect(neighbourBefore(shown, 'D', 'down')).toBeNull();
		expect(neighbourBefore(shown, 'H', 'up')).toBeNull();
	});
});

describe('dropBefore', () => {
	// Cards 40px tall with an 8px gap; the header sits above y=0.
	const boxes = [
		{ key: 'A', top: 0, height: 40 },
		{ key: 'B', top: 48, height: 40 },
		{ key: 'C', top: 96, height: 40 }
	];

	it('lands a drop on the header or in the top half of the first card before the first card', () => {
		expect(dropBefore(-20, boxes)).toBe('A');
		expect(dropBefore(10, boxes)).toBe('A');
	});

	it('lands a drop in the gap between two cards before the lower card, not at the end', () => {
		expect(dropBefore(44, boxes)).toBe('B');
		expect(dropBefore(92, boxes)).toBe('C');
	});

	it('uses the midpoint within a card and lands below the last card at the end', () => {
		expect(dropBefore(67, boxes)).toBe('B');
		expect(dropBefore(69, boxes)).toBe('C');
		expect(dropBefore(130, boxes)).toBeNull();
		expect(dropBefore(10, [])).toBeNull();
	});
});

describe('columnHeading', () => {
	it.each([
		['todo', 14, 'up next'],
		['in_progress', 14, 'in progress'],
		['done', 14, 'done · last 14 days'],
		['done', 1, 'done · last 1 day'],
		['done', undefined, 'done'],
		['done', 0, 'done']
	] as const)('%s with window %s reads %s', (col, days, want) => {
		expect(columnHeading(col, days)).toBe(want);
	});
});

describe('splitUpNext', () => {
	const item = (key: string, rank: number, ready: boolean) =>
		sw(key, { rank, delivery: 'todo' }, ready ? [] : [{ reason: 'no_criteria' }]);

	it('puts ready todo stories first and keeps rank order inside both parts', () => {
		const todo = [item('ABC-3', 1, false), item('ABC-1', 2, true), item('ABC-2', 3, false), item('ABC-4', 4, true)];
		const { ready, notReady } = splitUpNext(todo);
		expect(ready.map((s) => s.story.key)).toEqual(['ABC-1', 'ABC-4']);
		expect(notReady.map((s) => s.story.key)).toEqual(['ABC-3', 'ABC-2']);
	});

	it('a drop at the end of the ready part changes nothing there, though the full column would reorder', () => {
		// The board measures and checks a todo drop against the dragged card's own part (the page's dropOnColumn).
		const todo = [item('ABC-1', 1, true), item('ABC-4', 2, true), item('ABC-3', 3, false)];
		const { ready, notReady } = splitUpNext(todo);
		expect(orderAfterDrop(ready, 'ABC-4', null)).toBeNull();
		expect(orderAfterDrop(todo, 'ABC-4', null)).toEqual(['ABC-1', 'ABC-3', 'ABC-4']);
		expect(orderAfterDrop(notReady, 'ABC-3', null)).toBeNull();
	});
});

describe('afterPart', () => {
	const item = (key: string, rank: number, ready: boolean) =>
		sw(key, { rank, delivery: 'todo' }, ready ? [] : [{ reason: 'no_criteria' }]);

	it('names the full-column story after the part, so an end-of-part move stays inside the part', () => {
		const full = [item('ABC-1', 1, true), item('ABC-5', 2, false), item('ABC-4', 3, true), item('ABC-3', 4, false)];
		const { ready, notReady } = splitUpNext(full);
		expect(afterPart(full, ready)).toBe('ABC-3');
		expect(afterPart(full, notReady)).toBeNull();
	});

	it('follows the last shown card of the part, hidden cards included in the full column', () => {
		const full = [item('ABC-1', 1, true), item('ABC-2', 2, true), item('ABC-3', 3, false)];
		// ABC-2 is filtered out: the part shows only ABC-1, so the story after it in the full column is ABC-2.
		expect(afterPart(full, [full[0]])).toBe('ABC-2');
	});

	it('is null for an empty part or a part whose last card is not in the column', () => {
		const full = [item('ABC-1', 1, true)];
		expect(afterPart(full, [])).toBeNull();
		expect(afterPart(full, [item('ABC-9', 9, true)])).toBeNull();
	});
});

const statusItem = (delivery: Story['delivery'], reasons: Reason[] = [], resolution: Story['resolution'] = null) =>
	withReadiness({ key: 'S1', delivery, resolution }, reasons);

describe('storyStatus', () => {
	it('is ready from the server readiness', () => {
		expect(storyStatus(statusItem('todo'))).toEqual({ word: 'ready', tone: 'ready' });
	});
	it('names the first of several distinct reasons, in its exact short label', () => {
		const item = sw('S1', {}, [{ reason: 'blocked_by', story: 'ABC-9', delivery: 'todo', owner: null }, { reason: 'no_criteria' }, { reason: 'untriaged' }]);
		expect(storyStatus(item)).toEqual({ word: 'blocked by ABC-9 (todo)', tone: 'warn' });
		const other = sw('S2', {}, [{ reason: 'no_criteria' }, { reason: 'untriaged' }]);
		expect(storyStatus(other)).toEqual({ word: 'no criteria', tone: 'warn' });
	});
	it('shows an active story the server reports not ready by its reason, not as ready', () => {
		const item = sw('S1', { delivery: 'in_progress' }, [{ reason: 'decision_unchecked', decision_id: 7, title: 'Use Postgres' }]);
		expect(storyStatus(item)).toEqual({ word: 'decision unchecked', tone: 'warn' });
	});
	it('shows a done story by its resolution', () => {
		expect(storyStatus(statusItem('done', [], 'dropped'))).toEqual({ word: 'dropped', tone: 'muted' });
		expect(storyStatus(statusItem('done'))).toEqual({ word: 'done', tone: 'muted' });
	});
});
