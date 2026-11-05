import { describe, expect, it } from 'vitest';
import { storyHref } from './drawer.ts';

describe('storyHref', () => {
	it('keeps the path and the other parameters, and sets or replaces story', () => {
		expect(storyHref(new URL('http://localhost/backlog?mine=1'), 'RV-5')).toBe('/backlog?mine=1&story=RV-5');
		expect(storyHref(new URL('http://localhost/board?story=RV-5'), 'RV-6')).toBe('/board?story=RV-6');
		expect(storyHref(new URL('http://localhost/epics/ABC-5#x'), 'A B')).toBe('/epics/ABC-5?story=A+B');
	});
});
