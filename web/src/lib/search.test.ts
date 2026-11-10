import { describe, expect, it } from 'vitest';
import { story } from '#lib/test/fixtures.ts';
import { highlight, hitHref, hitLevel, isKeyShaped, matchedBy } from './search.ts';

const hit = (item: 'initiative' | 'epic' | 'story', kind: 'story' | 'bug' | 'chore' | null = null) => ({
	key: 'ABC-7',
	item,
	kind,
	title: 't',
	matched: ['title'],
	exact: false
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

describe('isKeyShaped', () => {
	it.each(['12', '#12', 'TST-12', 'tst-12', 'SCRUM-40', ' GRA-7 '])('%s names a key or a Jira reference', (q) => {
		expect(isKeyShaped(q)).toBe(true);
	});
	it.each(['invoice', '12 apples', 'TST-', '-12', 'a-b', '#', '', 'TST-12-3', '1-2'])('%s is text', (q) => {
		expect(isKeyShaped(q)).toBe(false);
	});
});

describe('matchedBy', () => {
	const s = story({ key: 'ABC-12', title: 'Invoice export', description: 'Sends the ZEBRA file', external_ref: 'SCRUM-40' });
	it('is null without text or when the title matches (the title is highlighted instead)', () => {
		expect(matchedBy(s, null, 'ABC')).toBeNull();
		expect(matchedBy(s, 'invoice', 'ABC')).toBeNull();
	});
	it('names the field the text matched otherwise', () => {
		expect(matchedBy(s, 'zebra', 'ABC')).toBe('matched description');
		expect(matchedBy(s, 'scrum-4', 'ABC')).toBe('matched SCRUM-40');
		expect(matchedBy(s, '12', 'ABC')).toBe('matched key');
		expect(matchedBy(s, '#12', 'ABC')).toBe('matched key');
		expect(matchedBy(s, 'abc-12', 'ABC')).toBe('matched key');
	});
	it('says "matched key" only for text that names the row\'s key; otherwise no tag', () => {
		// A bare number names a key under the current prefix, as the server reads it.
		expect(matchedBy(s, '12', 'OLD')).toBeNull();
		expect(matchedBy(s, '1', 'ABC')).toBeNull();
		expect(matchedBy(s, 'abc-1', 'ABC')).toBeNull();
		expect(matchedBy(s, '012', 'ABC')).toBeNull();
		// A match the client cannot see (a field the row does not carry): no guess.
		expect(matchedBy(s, 'pdf', 'ABC')).toBeNull();
	});
	it('matches as highlight does (one escaped iu RegExp): a title highlight never comes with a tag', () => {
		// Under the u flag 'ſ' (long s) folds to 's'; toLowerCase leaves it alone.
		const t = story({ key: 'ABC-1', title: 'ſtar chart', description: 'about ſtars' });
		expect(highlight(t.title, 'star').some((p) => p.match)).toBe(true);
		expect(matchedBy(t, 'star', 'ABC')).toBeNull();
		expect(matchedBy({ ...t, title: 'x' }, 'STAR', 'ABC')).toBe('matched description');
		// Regex characters are literal.
		expect(matchedBy(story({ key: 'ABC-1', title: 'x', description: 'a.b' }), 'a?b', 'ABC')).toBeNull();
		expect(matchedBy(story({ key: 'ABC-1', title: 'x', description: 'a.b' }), 'a.b', 'ABC')).toBe('matched description');
	});
});
