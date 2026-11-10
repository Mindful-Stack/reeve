import { describe, expect, it } from 'vitest';
import { decisionHref, storyHref } from './drawer.ts';

describe('storyHref', () => {
	it('keeps the path and the other parameters, and sets or replaces story', () => {
		expect(storyHref(new URL('http://localhost/backlog?mine=1'), 'RV-5')).toBe('/backlog?mine=1&story=RV-5');
		expect(storyHref(new URL('http://localhost/board?story=RV-5'), 'RV-6')).toBe('/board?story=RV-6');
		expect(storyHref(new URL('http://localhost/epics/ABC-5#x'), 'A B')).toBe('/epics/ABC-5?story=A+B');
	});
});

describe('the two drawers\' parameters', () => {
	it('decisionHref swaps story for decision and keeps the rest', () => {
		expect(decisionHref(new URL('http://x/board?story=A-1&notready=1'), 7)).toBe('/board?notready=1&decision=7');
	});
	it('storyHref drops decision', () => {
		expect(storyHref(new URL('http://x/board?decision=7'), 'A-1')).toBe('/board?story=A-1');
	});
});
