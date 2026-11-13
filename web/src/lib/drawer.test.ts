import { describe, expect, it } from 'vitest';
import { decisionHref, hasDrawer, storyHref } from './drawer.ts';

describe('storyHref', () => {
	it('keeps the path and the other parameters, and sets or replaces story', () => {
		expect(storyHref(new URL('http://localhost/backlog?mine=1'), 'RV-5')).toBe('/backlog?mine=1&story=RV-5');
		expect(storyHref(new URL('http://localhost/board?story=RV-5'), 'RV-6')).toBe('/board?story=RV-6');
		expect(storyHref(new URL('http://localhost/epics/ABC-5#x'), 'A B')).toBe('/epics/ABC-5?story=A+B#x');
	});
});

describe('the two drawers\' parameters', () => {
	it('decisionHref swaps story for decision and keeps the rest', () => {
		expect(decisionHref(new URL('http://x/board?story=A-1&notready=1'), 7)).toBe('/board?notready=1&decision=7');
	});
	it('both keep the hash, so a lane link keeps its lane', () => {
		expect(decisionHref(new URL('http://x/decisions#next-sprint'), 1)).toBe('/decisions?decision=1#next-sprint');
		expect(storyHref(new URL('http://x/decisions?decision=1#next-sprint'), 'A-1')).toBe('/decisions?story=A-1#next-sprint');
	});
	it('storyHref drops decision', () => {
		expect(storyHref(new URL('http://x/board?decision=7'), 'A-1')).toBe('/board?story=A-1');
	});
});

describe('hasDrawer', () => {
	it('is true for a story key or a positive whole decision id, whichever the layout would open', () => {
		expect(hasDrawer(new URL('http://x/board?story=A-1'))).toBe(true);
		expect(hasDrawer(new URL('http://x/board?decision=7'))).toBe(true);
	});
	it('is false for what opens nothing: no parameter, an empty story, a decision that is not a positive whole number', () => {
		for (const q of ['', '?story=', '?decision=', '?decision=abc', '?decision=0', '?decision=-1', '?decision=1.5']) {
			expect(hasDrawer(new URL('http://x/board' + q)), q).toBe(false);
		}
	});
});
