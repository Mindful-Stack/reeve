import { describe, expect, it } from 'vitest';
import { highlight, hitHref, hitLevel } from './search.ts';

const hit = (item: 'initiative' | 'epic' | 'story', kind: 'story' | 'bug' | 'chore' | null = null) => ({
	key: 'ABC-7',
	item,
	kind,
	title: 't',
	matched: ['title']
});

describe('search hits', () => {
	it('open where each level lives', () => {
		expect(hitHref(hit('initiative'))).toBe('/initiatives/ABC-7');
		expect(hitHref(hit('epic'))).toBe('/epics/ABC-7');
		expect(hitHref(hit('story', 'bug'))).toBe('/board?story=ABC-7');
	});
	it('take a story kind as the level', () => {
		expect(hitLevel(hit('story', 'bug'))).toBe('bug');
		expect(hitLevel(hit('epic'))).toBe('epic');
	});
});

describe('highlight', () => {
	it('marks every occurrence case-insensitively and keeps the original case', () => {
		expect(highlight('NorthCloud to northcloud', 'NORTHCLOUD')).toEqual([
			{ text: 'NorthCloud', match: true },
			{ text: ' to ', match: false },
			{ text: 'northcloud', match: true }
		]);
	});
	it('treats regex characters literally', () => {
		expect(highlight('a (b.c) d', '(b.c)')).toEqual([
			{ text: 'a ', match: false },
			{ text: '(b.c)', match: true },
			{ text: ' d', match: false }
		]);
		expect(highlight('abc', 'a.c')).toEqual([{ text: 'abc', match: false }]);
		expect(highlight('x a-b/c{2}] y', 'a-b/c{2}]')).toEqual([
			{ text: 'x ', match: false },
			{ text: 'a-b/c{2}]', match: true },
			{ text: ' y', match: false }
		]);
	});
	it('still marks a match when lower-casing changes the length of another part of the title', () => {
		// 'İ'.toLowerCase() is two code units, so offsets in the folded title no longer line up with the original.
		const pieces = highlight('İstanbul NorthCloud', 'northcloud');
		expect(pieces).toEqual([
			{ text: 'İstanbul ', match: false },
			{ text: 'NorthCloud', match: true }
		]);
		expect(pieces.map((p) => p.text).join('')).toBe('İstanbul NorthCloud');
		expect(highlight('NorthCloud İ northcloud', 'NORTHCLOUD').map((p) => p.text).join('')).toBe('NorthCloud İ northcloud');
	});
	it('returns the title whole for an empty query or no match', () => {
		expect(highlight('Title', '  ')).toEqual([{ text: 'Title', match: false }]);
		expect(highlight('Title', 'zz')).toEqual([{ text: 'Title', match: false }]);
	});
});
