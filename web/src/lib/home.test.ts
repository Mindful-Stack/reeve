import { describe, expect, it } from 'vitest';
import type { QueueLane } from '#lib/api/types.ts';
import { myDecisions, pickNext, sprintLine, underWay } from './home.ts';
import { decision, queueEntry, sprint, withReadiness } from '#lib/test/fixtures.ts';

const ME = 'me@example.com';
// The decider is padded and upper-cased: a match must compare as `sameEmail` does, not byte for byte.
const mine = (id: number, lane: QueueLane) => queueEntry({ lane, decision: decision({ id, decider: ' ME@example.com ' }) });

describe('pickNext', () => {
	it('lists urgent, then goal work, then the rest, each in the order given, and leaves out owned stories', () => {
		const plain = withReadiness({ key: 'P-1' });
		const goal = withReadiness({ key: 'G-1', goals: [{ id: 3, outcome: 'Ship it', via: 'story' }] });
		const urgent = withReadiness({ key: 'U-1', urgent_sources: [{ item: 'story', key: 'U-1' }] });
		const owned = withReadiness({ key: 'O-1', owner: 'x@example.com', urgent_sources: [{ item: 'story', key: 'O-1' }] });
		const plain2 = withReadiness({ key: 'P-2' });
		expect(pickNext([plain, goal, owned, urgent, plain2]).map((s) => s.story.key)).toEqual(['U-1', 'G-1', 'P-1', 'P-2']);
	});

	it('an urgent story serving a goal is listed once, with the urgent ones', () => {
		const both = withReadiness({ key: 'B-1', urgent_sources: [{ item: 'story', key: 'B-1' }], goals: [{ id: 3, outcome: 'Ship it', via: 'story' }] });
		const goal = withReadiness({ key: 'G-1', goals: [{ id: 3, outcome: 'Ship it', via: 'story' }] });
		expect(pickNext([goal, both]).map((s) => s.story.key)).toEqual(['B-1', 'G-1']);
	});

	it('shows at most the limit, the first in order', () => {
		const rows = Array.from({ length: 7 }, (_, i) => withReadiness({ key: `P-${i}` }));
		expect(pickNext(rows).map((s) => s.story.key)).toEqual(['P-0', 'P-1', 'P-2', 'P-3', 'P-4']);
	});
});

describe('underWay', () => {
	it('keeps in progress, in review and testing as started in rank order, and counts todo and ready todo', () => {
		// The server lists by delivery state, then rank; Home lists the started ones by rank alone.
		const rows = [
			withReadiness({ key: 'C', delivery: 'in_progress', rank: 30 }),
			withReadiness({ key: 'A', delivery: 'in_review', rank: 10 }),
			withReadiness({ key: 'B', delivery: 'testing', rank: 20 }),
			withReadiness({ key: 'D' }),
			withReadiness({ key: 'E' }, [{ reason: 'no_criteria' }])
		];
		const u = underWay(rows);
		expect(u.started.map((s) => s.story.key)).toEqual(['A', 'B', 'C']);
		expect(u.todo).toBe(2);
		expect(u.ready).toBe(1);
	});
});

describe('myDecisions', () => {
	it('lists this and next sprint in queue order (a nested decision by its row lane), counts my records to review in any lane and the rest, and ignores other deciders', () => {
		const other = queueEntry({ lane: 'this_sprint', decision: decision({ id: 9, decider: 'other@example.com' }) });
		const review = { kind: 'review_record', pr_url: 'https://github.com/o/r/pull/1' } as const;
		const d = myDecisions(
			[
				mine(3, 'next_sprint'),
				queueEntry({ lane: 'this_sprint', under: 'ABC-7', next_step: review, decision: decision({ id: 1, decider: ' ME@example.com ' }) }),
				queueEntry({ lane: 'beyond', needed_in: 'beyond', needed_in_reason: { kind: 'held' }, decision: decision({ id: 4, decider: 'me@example.com' }) }),
				queueEntry({ lane: 'beyond', next_step: review, decision: decision({ id: 2, decider: 'me@example.com' }) }),
				mine(5, 'open'),
				mine(6, 'satisfied'),
				other
			],
			ME
		);
		expect(d.listed.map((e) => e.decision.id)).toEqual([3, 1]);
		expect(d.review).toBe(2);
		expect(d.later).toBe(3);
	});
});

describe('sprintLine', () => {
	const s = sprint({ starts_on: '2026-11-10', ends_on: '2026-11-24' });
	it('counts the days left', () => expect(sprintLine(s, '2026-11-11')).toBe('Sprint 10–24 Nov · 13 days left'));
	it('says one day in the singular', () => expect(sprintLine(s, '2026-11-23')).toBe('Sprint 10–24 Nov · 1 day left'));
	it('says ends today on the end date', () => expect(sprintLine(s, '2026-11-24')).toBe('Sprint 10–24 Nov · ends today'));
	it('says the end date passed, never that the sprint ended', () =>
		expect(sprintLine(s, '2026-11-26')).toBe('Sprint 10–24 Nov · end date passed 2 days ago'));
	it('names both months when the sprint spans two', () =>
		expect(sprintLine(sprint({ starts_on: '2026-11-24', ends_on: '2026-12-08' }), '2026-11-24')).toBe('Sprint 24 Nov–8 Dec · 14 days left'));
});
