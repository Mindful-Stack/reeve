import { describe, expect, it } from 'vitest';
import type { Reason, RequiredDecision, Story } from '#lib/api/types.ts';
import { decision as decisionDto, withReadiness } from '#lib/test/fixtures.ts';
import { epicCounts, epicStatus, stripDecisions, sumCounts, waitingOn } from './initiative.ts';

/** A story with only the reasons that need no more than their name, as the counts read them. */
type Simple = Extract<Reason, { reason: 'untriaged' | 'no_criteria' }>['reason'];
const story = (key: string, epic: string | null, delivery: Story['delivery'], reasons: Simple[] = []) =>
	withReadiness({ key, epic, delivery }, reasons.map((reason) => ({ reason })));

const decision = (id: number, on: string, title: string, extra: Partial<RequiredDecision> = {}) =>
	decisionDto({ id, on, title, ...extra });

describe('epicCounts', () => {
	it('counts each epic from delivery and the server readiness, every listed epic present', () => {
		const m = epicCounts(
			[
				story('S1', 'E1', 'todo'),
				story('S2', 'E1', 'todo', ['no_criteria']),
				story('S3', 'E1', 'todo', ['untriaged']),
				story('S4', 'E1', 'in_progress'),
				story('S5', 'E1', 'in_review'),
				story('S6', 'E1', 'testing', ['no_criteria']),
				story('S7', 'E1', 'done'),
				story('S8', 'E9', 'todo'),
				story('S9', null, 'todo')
			],
			['E1', 'E2']
		);
		expect(m.get('E1')).toEqual({ total: 7, done: 1, inProgress: 3, readyTodo: 1, needCriteria: 1 });
		expect(m.get('E2')).toEqual({ total: 0, done: 0, inProgress: 0, readyTodo: 0, needCriteria: 0 });
		expect(m.has('E9')).toBe(false);
	});

	it('sums epics for the initiative', () => {
		const a = { total: 3, done: 1, inProgress: 1, readyTodo: 1, needCriteria: 0 };
		const b = { total: 2, done: 2, inProgress: 0, readyTodo: 0, needCriteria: 1 };
		expect(sumCounts([a, b])).toEqual({ total: 5, done: 3, inProgress: 1, readyTodo: 1, needCriteria: 1 });
		expect(sumCounts([])).toEqual({ total: 0, done: 0, inProgress: 0, readyTodo: 0, needCriteria: 0 });
	});
});

describe('epicStatus', () => {
	const c = { total: 6, done: 0, inProgress: 2, readyTodo: 4, needCriteria: 0 };
	it('says what the epic waits on first, then criteria, then the flow', () => {
		expect(epicStatus({ ...c, needCriteria: 3 }, 'Email provider')).toBe('Waiting on Email provider');
		expect(epicStatus({ ...c, needCriteria: 3 }, null)).toBe('3 need criteria');
		expect(epicStatus({ ...c, needCriteria: 1 }, null)).toBe('1 needs criteria');
		expect(epicStatus(c, null)).toBe('4 ready · 2 in progress');
	});
	it('names an empty epic and a finished one', () => {
		expect(epicStatus({ total: 0, done: 0, inProgress: 0, readyTodo: 0, needCriteria: 0 }, null)).toBe('No stories yet');
		expect(epicStatus({ total: 2, done: 2, inProgress: 0, readyTodo: 0, needCriteria: 0 }, null)).toBe('All done');
	});
});

describe('decisions on the initiative page', () => {
	const list = [
		decision(1, 'E1', 'Email provider'),
		decision(2, 'E1', 'Later one'),
		decision(3, 'E2', 'Accepted', { check: { kind: 'accepted', commit: 'abcdef0123' } }),
		decision(4, 'E2', 'Closed', { closed: { by: 'a@b', at: '2026-10-01T00:00:00Z', reason: 'x' } }),
		decision(5, 'INI', 'Scope'),
		decision(6, 'S1', 'On a story')
	];
	it('maps each epic to the first blocking decision on it, else the initiative\'s', () => {
		const w = waitingOn(list, 'INI');
		expect(w('E1')).toBe('Email provider');
		expect(w('E2')).toBe('Scope'); // its accepted and closed decisions do not block; the initiative's does
		expect(waitingOn(list.filter((d) => d.on !== 'INI'), 'INI')('E2')).toBeNull();
	});
	it('names the strip decisions by the initiative or epic they hold up, not stories', () => {
		const d = stripDecisions(list, { key: 'INI', title: 'Orion' }, [{ key: 'E1', title: 'Email' }, { key: 'E2', title: 'Hosting' }]);
		expect(d).toEqual([{ onTitle: 'Email' }, { onTitle: 'Email' }, { onTitle: 'Orion' }]);
	});
});
