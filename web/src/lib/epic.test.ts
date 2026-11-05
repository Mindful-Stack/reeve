import { describe, expect, it } from 'vitest';
import type { RequiredDecision } from '#lib/api/types.ts';
import { decision as decisionDto, withReadiness } from '#lib/test/fixtures.ts';
import { epicStripDecisions, pollsChecks } from './epic.ts';

const decision = (id: number, on: string, extra: Partial<RequiredDecision> = {}) =>
	decisionDto({ id, on, title: `D${id}`, ...extra });
const closed = { at: '2026-10-20T00:00:00Z', by: 'a@example.com', reason: 'decided elsewhere' };
const record: RequiredDecision['record'] = { repo: 'kb', path: 'adrs/ADR-0001.md', record_id: 'ADR-0001' };
const epic = { key: 'E1', title: 'Login', initiative: 'I1' };

describe('epicStripDecisions', () => {
	it('names the epic for its own blocking decisions and the initiative for those on it', () => {
		const out = epicStripDecisions(
			[decision(1, 'E1'), decision(2, 'E1', { check: { kind: 'accepted', commit: 'abc1234' } })],
			[decision(3, 'I1'), decision(4, 'E2'), decision(5, 'E1'), decision(6, 'S1')],
			epic,
			'Orion'
		);
		expect(out).toEqual([{ onTitle: 'Login' }, { onTitle: 'Orion' }]);
	});

	it('ignores closed decisions and falls back to the initiative key without a title', () => {
		const out = epicStripDecisions(
			[decision(1, 'E1', { closed })],
			[decision(3, 'I1')],
			epic,
			null
		);
		expect(out).toEqual([{ onTitle: 'I1' }]);
	});

	it('reads nothing from the initiative list for an epic under no initiative', () => {
		expect(epicStripDecisions([], [decision(3, 'I1')], { ...epic, initiative: null }, 'Orion')).toEqual([]);
	});
});

describe('pollsChecks', () => {
	const rec = (id: number, on: string, extra: Partial<RequiredDecision> = {}) => decision(id, on, { record, ...extra });
	const stories = [withReadiness({ key: 'S1' })];

	it('polls for an open decision with a record on the epic, one of its stories or the initiative itself', () => {
		expect(pollsChecks([rec(1, 'E1')], [], epic, stories)).toBe(true);
		expect(pollsChecks([], [rec(2, 'S1')], epic, stories)).toBe(true);
		expect(pollsChecks([], [rec(3, 'I1')], epic, stories)).toBe(true);
	});

	it("does not poll for a sibling epic's decision, a closed one or one without a record", () => {
		expect(pollsChecks([], [rec(4, 'E2'), rec(5, 'S9')], epic, stories)).toBe(false);
		expect(pollsChecks([rec(1, 'E1', { closed })], [], epic, stories)).toBe(false);
		expect(pollsChecks([decision(1, 'E1')], [], epic, stories)).toBe(false);
	});
});
