// The rules of the Relations component: grouping by type and side, the keys a removal names, where a linked item
// opens, a web link's host, when the informational line shows, and the Add form's hint for each name.
import { describe, expect, it } from 'vitest';
import { itemLink } from '#lib/test/fixtures.ts';
import { followsNote, groupLinks, LINK_TYPES, linkedHref, linkTypeHint, linkTypeText, removal, webHost } from './relations.ts';

const to = (key: string, level: 'initiative' | 'epic' | 'story' = 'story') => ({ key, level, title: `title of ${key}` });

describe('LINK_TYPES', () => {
	it('offers the three names in display order, worded with spaces', () => {
		expect(LINK_TYPES.map((t) => [t, linkTypeText(t)])).toEqual([
			['follows', 'follows'],
			['duplicates', 'duplicates'],
			['relates_to', 'relates to']
		]);
	});
});

describe('linkTypeHint', () => {
	it('says what follows states, from this item, in the words of its level, and how to say the reverse', () => {
		expect(linkTypeHint('follows', 'epic')).toBe('This epic comes after the item you pick. To say what comes after this epic, add the item link from that item.');
		expect(linkTypeHint('follows', 'story')).toBe('This story comes after the item you pick. To say what comes after this story, add the item link from that item.');
	});
	it('says what duplicates states', () => {
		expect(linkTypeHint('duplicates', 'initiative')).toBe('This initiative covers the same work as the item you pick.');
		expect(linkTypeHint('duplicates', 'story')).toBe('This story covers the same work as the item you pick.');
	});
	it('has no hint for relates_to: it reads the same from both sides', () => {
		expect(linkTypeHint('relates_to', 'epic')).toBe('');
	});
});

describe('groupLinks', () => {
	it('groups by type and direction in the order the server sent, whatever the labels say', () => {
		const groups = groupLinks([
			itemLink({ type: 'follows', direction: 'out', label: 'follows', item: to('ABC-2') }),
			itemLink({ type: 'follows', direction: 'in', label: 'same words', item: to('ABC-3') }),
			itemLink({ type: 'duplicates', direction: 'in', label: 'same words', item: to('ABC-4') }),
			itemLink({ type: 'follows', direction: 'out', label: 'another wording', item: to('ABC-5') })
		]);
		expect(groups.map((g) => [g.id, g.label, g.links.map((l) => l.item.key)])).toEqual([
			['follows-out', 'follows', ['ABC-2', 'ABC-5']],
			['follows-in', 'same words', ['ABC-3']],
			['duplicates-in', 'same words', ['ABC-4']]
		]);
	});
	it('has no group for no links', () => {
		expect(groupLinks([])).toEqual([]);
	});
});

describe('removal', () => {
	it('names this item first for an out link', () => {
		expect(removal('ABC-10', itemLink({ type: 'duplicates', direction: 'out', item: to('ABC-7') }))).toEqual({ key: 'ABC-10', type: 'duplicates', to: 'ABC-7' });
	});
	it('swaps the two keys for an in link: writes are always in the forward form', () => {
		expect(removal('ABC-10', itemLink({ type: 'follows', direction: 'in', label: 'is followed by', item: to('ABC-12') }))).toEqual({ key: 'ABC-12', type: 'follows', to: 'ABC-10' });
	});
	it('sends relates_to as shown: it has one side', () => {
		expect(removal('ABC-10', itemLink({ type: 'relates_to', direction: 'out', label: 'relates to', item: to('ABC-3') }))).toEqual({ key: 'ABC-10', type: 'relates_to', to: 'ABC-3' });
	});
});

describe('linkedHref', () => {
	const url = { href: 'http://localhost/epics/ABC-2?decision=4#stories' };
	it('opens a story in the drawer over the current view, and an epic or an initiative on its page', () => {
		expect(linkedHref(url, to('ABC-11'))).toBe('/epics/ABC-2?story=ABC-11#stories');
		expect(linkedHref(url, to('ABC-5', 'epic'))).toBe('/epics/ABC-5');
		expect(linkedHref(url, to('ABC-1', 'initiative'))).toBe('/initiatives/ABC-1');
	});
});

describe('webHost', () => {
	it('is the host of an http or https URL, with its port', () => {
		expect(webHost('https://example.com/wiki/scenario?x=1')).toBe('example.com');
		expect(webHost('http://wiki.example.com:8443/a')).toBe('wiki.example.com:8443');
	});
	it('is null for anything that must not become a live link', () => {
		for (const bad of ['javascript:alert(1)', 'data:text/html,x', 'ftp://example.com/file', 'example.com/wiki', '']) expect(webHost(bad), bad).toBeNull();
	});
});

describe('followsNote', () => {
	it('names both stories for a follows between two stories', () => {
		expect(followsNote('follows', 'ABC-10', 'story', { key: 'ABC-11', item: 'story' })).toEqual({ key: 'ABC-10', by: 'ABC-11' });
	});
	it('is null for any other name and for any end that is not a story', () => {
		expect(followsNote('duplicates', 'ABC-10', 'story', { key: 'ABC-11', item: 'story' })).toBeNull();
		expect(followsNote('relates_to', 'ABC-10', 'story', { key: 'ABC-11', item: 'story' })).toBeNull();
		expect(followsNote('follows', 'ABC-10', 'story', { key: 'ABC-5', item: 'epic' })).toBeNull();
		expect(followsNote('follows', 'ABC-5', 'epic', { key: 'ABC-11', item: 'story' })).toBeNull();
		expect(followsNote('follows', 'ABC-1', 'initiative', { key: 'ABC-5', item: 'epic' })).toBeNull();
	});
});
