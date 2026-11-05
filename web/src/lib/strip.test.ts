import { describe, expect, it } from 'vitest';
import { blockerCell } from './strip.ts';

describe('blockerCell', () => {
	it('names the blocking decisions first', () => {
		expect(blockerCell({ blockingDecisions: [{ onTitle: 'Email templates' }], storiesNeedingCriteria: 3, criteriaAnchor: '#stories' }))
			.toEqual({ blocking: true, text: '1 blocking decision', detail: 'on Email templates', anchor: '#decisions' });
	});
	it('then stories without criteria, then nothing', () => {
		expect(blockerCell({ blockingDecisions: [], storiesNeedingCriteria: 3, criteriaAnchor: '#stories' })).toEqual({ blocking: true, text: '3 stories need criteria', detail: null, anchor: '#stories' });
		expect(blockerCell({ blockingDecisions: [], storiesNeedingCriteria: 1, criteriaAnchor: '#stories' }).text).toBe('1 story needs criteria');
		expect(blockerCell({ blockingDecisions: [], storiesNeedingCriteria: 0, criteriaAnchor: '#stories' })).toEqual({ blocking: false, text: 'Nothing blocking', detail: null, anchor: null });
	});
	it('links stories needing criteria to the caller\'s anchor', () => {
		expect(blockerCell({ blockingDecisions: [], storiesNeedingCriteria: 2, criteriaAnchor: '#epics' }).anchor).toBe('#epics');
	});
	it('caps the places at two, then says how many more', () => {
		const d = ['A', 'B', 'C', 'D'].map((onTitle) => ({ onTitle }));
		expect(blockerCell({ blockingDecisions: d, storiesNeedingCriteria: 0, criteriaAnchor: '#stories' }).detail).toBe('on A, B and 2 more');
	});
});
