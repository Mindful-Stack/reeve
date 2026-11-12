import { describe, expect, it } from 'vitest';
import type { GoalProgress, SprintCounts } from '#lib/api/types.ts';
import {
	promisesHeading,
	blockedText,
	confidenceLabel,
	confidenceTone,
	countsDiffer,
	goalFilter,
	nowThen,
	nudgeStillDue,
	RULE,
	shouldNudge,
	truncate,
	upkeepText,
	upkeepTextAt,
	verdictLabel,
	warningText
} from './sprint.ts';

describe('shouldNudge', () => {
	const story = (delivery: string, goals: unknown[] = []) => ({ delivery, goals }) as never;
	it('nudges a start from todo or done that serves no goal while a sprint is active', () => {
		expect(shouldNudge('todo', story('in_progress'), true)).toBe(true);
		expect(shouldNudge('done', story('testing'), true)).toBe(true);
	});
	it('stays quiet for moves between active states, moves into done, served stories, or no active sprint', () => {
		expect(shouldNudge('in_progress', story('in_review'), true)).toBe(false);
		expect(shouldNudge('in_progress', story('done'), true)).toBe(false);
		expect(shouldNudge('todo', story('in_progress', [{ id: 1 }]), true)).toBe(false);
		expect(shouldNudge('todo', story('in_progress'), false)).toBe(false);
		expect(shouldNudge('todo', story('done'), true)).toBe(false);
		expect(shouldNudge('done', story('todo'), true)).toBe(false);
	});
});

describe('upkeepText', () => {
	it('reads as a count of completions with the reference line, never a verdict', () => {
		expect(upkeepText({ completed: 24, upkeep_completed: 7 }, 20)).toBe(
			'7 of 24 completed stories were upkeep (29%; team line 20%)'
		);
		expect(upkeepText({ completed: 0, upkeep_completed: 0 }, 20)).toBe('nothing completed yet (team line 20%)');
	});
});

describe('countsDiffer', () => {
	const g = (goal_id: number, over: Partial<GoalProgress> = {}): GoalProgress => ({
		goal_id,
		completed: 2,
		in_flight: 1,
		todo: 3,
		in_review: 4,
		blocked_decisions: 5,
		...over
	});
	const c = (over: Partial<SprintCounts> = {}): SprintCounts => ({
		goals: [g(1), g(2)],
		completed: 2,
		upkeep_completed: 1,
		...over
	});

	it('is false for equal counts and for a missing entry', () => {
		expect(countsDiffer(c(), c())).toBe(false);
		expect(countsDiffer(c(), null)).toBe(false);
	});
	it('sees a changed total completed', () => {
		expect(countsDiffer(c({ completed: 3 }), c())).toBe(true);
	});
	it('sees a changed upkeep_completed', () => {
		expect(countsDiffer(c({ upkeep_completed: 2 }), c())).toBe(true);
	});
	it.each(['completed', 'in_flight', 'todo', 'in_review'] as const)('sees a changed goal %s', (field) => {
		expect(countsDiffer(c({ goals: [g(1, { [field]: 9 }), g(2)] }), c())).toBe(true);
	});
	it('ignores blocked_decisions, a live measure', () => {
		expect(countsDiffer(c({ goals: [g(1, { blocked_decisions: 0 }), g(2, { blocked_decisions: 9 })] }), c())).toBe(false);
	});
	it('ignores the order of goals and of keys', () => {
		const reordered = {
			upkeep_completed: 1,
			completed: 2,
			goals: [g(2), { blocked_decisions: 5, in_review: 4, todo: 3, in_flight: 1, completed: 2, goal_id: 1 }]
		};
		expect(countsDiffer(c(), reordered)).toBe(false);
	});
	it('sees a goal on one side only', () => {
		expect(countsDiffer(c({ goals: [g(1)] }), c())).toBe(true);
		expect(countsDiffer(c(), c({ goals: [g(1)] }))).toBe(true);
	});
	it('sees a different goal id at the same count', () => {
		expect(countsDiffer(c({ goals: [g(1), g(3)] }), c())).toBe(true);
	});
});

it('states the membership rule in one sentence', () => {
	expect(RULE).toBe('Counts the work completed in this sprint; work started but not finished shows as in flight.');
});

it('labels confidence', () => {
	expect(confidenceLabel('at_risk')).toBe('at risk');
});

describe('truncate', () => {
	it('keeps text within the limit and cuts longer text to the limit with an ellipsis', () => {
		expect(truncate('Ship the import', 40)).toBe('Ship the import');
		expect(truncate('a'.repeat(40), 40)).toBe('a'.repeat(40));
		const cut = truncate('a'.repeat(41), 40);
		expect(cut).toBe(`${'a'.repeat(39)}…`);
		expect(cut.length).toBe(40);
	});
});

describe('confidenceTone', () => {
	it('maps a recorded confidence to a dot tone, and none to muted', () => {
		expect(confidenceTone('on_track')).toBe('ready');
		expect(confidenceTone('at_risk')).toBe('warn');
		expect(confidenceTone('off_track')).toBe('danger');
		expect(confidenceTone(null)).toBe('muted');
		expect(confidenceTone(undefined)).toBe('muted');
	});
});

describe('nudgeStillDue', () => {
	const story = (delivery: string, goals: unknown[] = []) => ({ delivery, goals }) as never;
	it('holds while the story is in active work and serves no goal', () => {
		expect(nudgeStillDue(story('in_progress'))).toBe(true);
		expect(nudgeStillDue(story('testing'))).toBe(true);
		expect(nudgeStillDue(story('in_progress', [{ id: 1 }]))).toBe(false);
		expect(nudgeStillDue(story('todo'))).toBe(false);
		expect(nudgeStillDue(story('done'))).toBe(false);
	});
});

describe('goalFilter', () => {
	it('reads a goal of the active sprint or none; anything else, or no active sprint, filters nothing', () => {
		expect(goalFilter('1', [1, 2])).toBe(1);
		expect(goalFilter('none', [1, 2])).toBe('none');
		expect(goalFilter('99', [1, 2])).toBeNull();
		expect(goalFilter('x', [1, 2])).toBeNull();
		expect(goalFilter(null, [1, 2])).toBeNull();
		expect(goalFilter('1', null)).toBeNull();
		expect(goalFilter('none', null)).toBeNull();
	});
});

describe('nowThen', () => {
	it('shows one number unless the value at close differs, then both', () => {
		expect(nowThen(4, undefined)).toBe('4');
		expect(nowThen(4, 4)).toBe('4');
		expect(nowThen(4, 3)).toBe('now 4 · at close 3');
		expect(nowThen(0, 1)).toBe('now 0 · at close 1');
	});
});

it('says how many decisions block, in the singular for one', () => {
	expect(blockedText(1)).toBe('blocked by 1 decision');
	expect(blockedText(0)).toBe('blocked by 0 decisions');
	expect(blockedText(3)).toBe('blocked by 3 decisions');
});

it('labels verdicts and warnings as the CLI does', () => {
	expect(verdictLabel('partly_met')).toBe('partly met');
	expect(verdictLabel('missed')).toBe('missed');
	expect(warningText('more_than_three_goals')).toBe('more than three goals thins focus');
	expect(warningText('no_goals')).toBe('the sprint has no goals');
});

describe('upkeepTextAt', () => {
	it('reads as upkeepText unless a total changed since close, then the now sentence with the at-close figures in one parenthesis', () => {
		const now = { completed: 6, upkeep_completed: 2 };
		expect(upkeepTextAt(now, null, 20)).toBe(upkeepText(now, 20));
		expect(upkeepTextAt(now, { completed: 6, upkeep_completed: 2 }, 20)).toBe(upkeepText(now, 20));
		expect(upkeepTextAt(now, { completed: 5, upkeep_completed: 2 }, 20)).toBe(
			'2 of 6 completed stories were upkeep (at close: 2 of 5) · team line 20%'
		);
		expect(upkeepTextAt(now, { completed: 6, upkeep_completed: 3 }, 20)).toBe(
			'2 of 6 completed stories were upkeep (at close: 3 of 6) · team line 20%'
		);
		expect(upkeepTextAt({ completed: 0, upkeep_completed: 0 }, { completed: 1, upkeep_completed: 1 }, 20)).toBe(
			'0 of 0 completed stories were upkeep (at close: 1 of 1) · team line 20%'
		);
	});
});

describe('a closed sprint\'s promise dates', () => {
	it('promisesHeading: the day after the previous end to this end, the month once when shared', () => {
		expect(promisesHeading('2026-11-10', '2026-11-24')).toBe('Promises due 11–24 Nov');
		expect(promisesHeading('2026-10-27', '2026-11-10')).toBe('Promises due 28 Oct–10 Nov');
		expect(promisesHeading('2026-10-31', '2026-11-13')).toBe('Promises due 1–13 Nov');
		expect(promisesHeading('2026-12-31', '2027-01-13')).toBe('Promises due 1–13 Jan');
		expect(promisesHeading(null, '2026-10-27')).toBe('Promises due up to 27 Oct');
	});
});
