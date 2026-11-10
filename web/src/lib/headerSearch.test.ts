import { describe, expect, it, vi } from 'vitest';
import type { SearchHit } from '#lib/api/types.ts';
import { headerSearch, headerTarget } from './headerSearch.ts';

const hit = (key: string, item: SearchHit['item'], exact = true): SearchHit => ({ key, item, kind: item === 'story' ? 'story' : null, title: key, matched: ['key'], exact });
const at = (path: string) => new URL(`http://localhost${path}`);
const never = () => Promise.reject(new Error('no lookup expected'));

describe('headerTarget', () => {
	it('a key that names a story opens its drawer over the current page', async () => {
		const lookup = vi.fn(async () => [hit('GRA-12', 'story')]);
		expect(await headerTarget(' gra-12 ', at('/search?q=x'), lookup)).toEqual({ href: '/search?q=x&story=GRA-12', kind: 'story' });
		expect(lookup).toHaveBeenCalledWith('gra-12');
	});
	it('a key that names an epic or an initiative goes to its page', async () => {
		expect(await headerTarget('#3', at('/board'), async () => [hit('GRA-3', 'epic')])).toEqual({ href: '/epics/GRA-3', kind: 'item' });
		expect(await headerTarget('SCRUM-40', at('/board'), async () => [hit('GRA-4', 'initiative')])).toEqual({ href: '/initiatives/GRA-4', kind: 'item' });
	});
	it('a key with no exact first hit, or a failed lookup, filters the backlog by the text', async () => {
		const filter = { href: '/backlog?q=12&state=open&view=relevance', kind: 'filter' };
		expect(await headerTarget('12', at('/board'), async () => [hit('GRA-120', 'story', false)])).toEqual(filter);
		expect(await headerTarget('12', at('/board'), async () => [])).toEqual(filter);
		expect(await headerTarget('12', at('/board'), never)).toEqual(filter);
	});
	it('text off the backlog lands on the open backlog by relevance; text is never looked up', async () => {
		expect(await headerTarget('12 apples & pears', at('/board?mine=1'), never)).toEqual({
			href: '/backlog?q=12+apples+%26+pears&state=open&view=relevance',
			kind: 'filter'
		});
	});
	it('text on the backlog sets q and keeps every other param', async () => {
		expect(await headerTarget('zebra', at('/backlog?kind=bug&state=all&view=epic'), never)).toEqual({
			href: '/backlog?kind=bug&state=all&view=epic&q=zebra',
			kind: 'filter'
		});
		// A new text replaces the old one; no view is added once there was text.
		expect((await headerTarget('lion', at('/backlog?q=zebra&kind=bug'), never))?.href).toBe('/backlog?q=lion&kind=bug');
		// No view and no previous text: relevance.
		expect((await headerTarget('lion', at('/backlog?kind=bug'), never))?.href).toBe('/backlog?kind=bug&q=lion&view=relevance');
		// The legacy group=rank is a view.
		expect((await headerTarget('lion', at('/backlog?group=rank'), never))?.href).toBe('/backlog?group=rank&q=lion');
	});
	it('empty text goes nowhere off the backlog; on it, it drops the text and relevance with it', async () => {
		expect(await headerTarget('   ', at('/board'), never)).toBeNull();
		expect(await headerTarget('', at('/backlog?kind=bug'), never)).toBeNull();
		expect(await headerTarget(' ', at('/backlog?q=zebra&kind=bug&view=relevance'), never)).toEqual({ href: '/backlog?kind=bug', kind: 'filter' });
		expect((await headerTarget('', at('/backlog?q=zebra&view=rank'), never))?.href).toBe('/backlog?view=rank');
	});
});

describe('headerSearch', () => {
	it('navigates only for the latest submission, however the lookups resolve', async () => {
		const answers: ((h: SearchHit[]) => void)[] = [];
		const lookup = (_: string) => new Promise<SearchHit[]>((resolve) => answers.push(resolve));
		const navigate = vi.fn(async () => {});
		const busy: boolean[] = [];
		const submit = headerSearch(lookup, navigate, (b) => busy.push(b), () => 'http://localhost/board');
		const first = submit('GRA-1', at('/board'));
		const second = submit('GRA-2', at('/board'));
		answers[1]([hit('GRA-2', 'epic')]);
		await second;
		answers[0]([hit('GRA-1', 'epic')]);
		await first;
		expect(navigate.mock.calls).toEqual([[{ href: '/epics/GRA-2', kind: 'item' }]]);
		expect(busy).toEqual([true, true, false]);
	});
	it('a text submitted while a key lookup is pending wins over the lookup', async () => {
		let answer: (h: SearchHit[]) => void = () => {};
		const navigate = vi.fn(async () => {});
		const submit = headerSearch(() => new Promise((resolve) => (answer = resolve)), navigate, () => {}, () => 'http://localhost/board');
		const key = submit('GRA-1', at('/board'));
		await submit('zebra', at('/board'));
		answer([hit('GRA-1', 'epic')]);
		await key;
		expect(navigate.mock.calls).toEqual([[{ href: '/backlog?q=zebra&state=open&view=relevance', kind: 'filter' }]]);
	});
	it('drops an answer once the page has moved on since the submit, jump and fall-through alike', async () => {
		const answers: ((h: SearchHit[]) => void)[] = [];
		const navigate = vi.fn(async () => {});
		let href = 'http://localhost/board';
		const busy: boolean[] = [];
		const submit = headerSearch(() => new Promise((resolve) => answers.push(resolve)), navigate, (b) => busy.push(b), () => href);
		const jump = submit('GRA-1', at('/board'));
		href = 'http://localhost/sprint';
		answers[0]([hit('GRA-1', 'epic')]);
		await jump;
		const fallThrough = submit('GRA-2', at('/sprint'));
		href = 'http://localhost/sprint?story=GRA-9';
		answers[1]([]);
		await fallThrough;
		expect(navigate).not.toHaveBeenCalled();
		expect(busy).toEqual([true, false, true, false]);
		// Unchanged, it navigates.
		const kept = submit('GRA-3', at('/sprint?story=GRA-9'));
		answers[2]([hit('GRA-3', 'epic')]);
		await kept;
		expect(navigate.mock.calls).toEqual([[{ href: '/epics/GRA-3', kind: 'item' }]]);
	});
});
