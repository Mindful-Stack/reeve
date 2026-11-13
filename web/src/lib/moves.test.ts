import { describe, expect, it } from 'vitest';
import type { Reason } from '#lib/api/types.ts';
import { moveStep, refusal } from '#lib/moves.ts';

const blocked: Reason = { reason: 'blocked_by', story: 'ABC-2', delivery: 'todo', owner: null };
const rule: Reason = { reason: 'resolution_required' };

describe('moveStep', () => {
	it('asks for a resolution on a move into done', () => {
		expect(moveStep({ delivery: 'todo', resolution: null }, 'done')).toBe('resolve');
		expect(moveStep({ delivery: 'testing', resolution: null }, 'done')).toBe('resolve');
	});
	it('re-resolves a completed story within done, but not a dropped one', () => {
		expect(moveStep({ delivery: 'done', resolution: 'completed' }, 'done')).toBe('resolve');
		expect(moveStep({ delivery: 'done', resolution: 'dropped' }, 'done')).toBe('none');
	});
	it('moves to another column and does nothing for the same one', () => {
		expect(moveStep({ delivery: 'todo', resolution: null }, 'in_progress')).toBe('move');
		expect(moveStep({ delivery: 'in_review', resolution: null }, 'in_review')).toBe('none');
	});
});

describe('refusal', () => {
	it('offers a person the override when the server says one can get past the refusal', () => {
		expect(refusal('ABC-1', 'in_progress', [blocked], true, true)).toEqual({ kind: 'override', reasons: [blocked] });
	});
	it('gives automation a notice instead', () => {
		expect(refusal('ABC-1', 'in_progress', [blocked], true, false)).toEqual({
			kind: 'notice',
			notice: 'ABC-1 was not moved to in progress: blocked by ABC-2: todo, not yet started by anyone.'
		});
	});
	it('never offers an override for a delivery rule', () => {
		expect(refusal('ABC-1', 'done', [blocked, rule], false, true).kind).toBe('notice');
	});
	it('follows the flag the server sent, not its own reading of the reasons', () => {
		expect(refusal('ABC-1', 'in_progress', [blocked], false, true).kind).toBe('notice');
	});
});
