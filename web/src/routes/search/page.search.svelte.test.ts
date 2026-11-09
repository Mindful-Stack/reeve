// @vitest-environment jsdom
// The search page reads ?q=, lists each hit with its level glyph, a link to the item and where it matched.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { goto } from '$app/navigation';
import type { SearchHit } from '#lib/api/types.ts';
import type { FakePage } from '#lib/test/net.ts';
import Page from './+page.svelte';

const route = vi.hoisted(() => ({ fake: null as null | FakePage }));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	route.fake = fakePage({ url: '/search?q=northcloud' });
	return { page: route.fake.page };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn((to: string | URL) => route.fake!.goto(to)) }));

const net = vi.hoisted(() => ({ asked: [] as string[], mode: { fail: false, exactHits: false } }));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	installFetch(({ url }) => {
		net.asked.push(url.pathname + url.search);
		if (net.mode.fail) return failure('boom');
		const value: SearchHit[] = [
			{ key: 'ABC-3', item: 'story', kind: 'bug', title: 'Move the database', matched: ['description'], exact: false },
			{ key: 'ABC-1', item: 'initiative', kind: null, title: 'Leave NorthCloud now', matched: ['title'], exact: false }
		];
		// The server's order: the key, then an exact reference (a story, below the initiative's level), then the rest.
		if (net.mode.exactHits)
			value.unshift(
				{ key: 'ABC-12', item: 'story', kind: 'story', title: 'The one', matched: ['key'], exact: true },
				{ key: 'ABC-9', item: 'story', kind: 'bug', title: 'Migrated', matched: ['external_ref'], exact: true }
			);
		return value;
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.mode.fail = false;
	net.mode.exactHits = false;
	route.fake!.reset();
});

describe('search page', () => {
	it('searches the query in the URL and links each hit', async () => {
		app = mount(Page, { target: document.body });
		const items = await vi.waitFor(() => {
			flushSync();
			const li = [...document.body.querySelectorAll('li')];
			expect(li).toHaveLength(2);
			return li;
		});
		expect(net.asked).toContain('/api/v1/search?q=northcloud');
		expect((document.body.querySelector('input[type="search"]') as HTMLInputElement).value).toBe('northcloud');
		expect(items[1].querySelector('a')!.getAttribute('href')).toBe('/board?story=ABC-3');
		expect(items[1].textContent).toContain('▲');
		expect(items[1].textContent).toContain('Move the database');
		expect(items[1].textContent).toContain('description');
		expect([...document.body.querySelectorAll('h2')].map((h) => h.textContent?.trim())).toEqual(['Initiatives 1', 'Stories 1']);
		expect(items[0].querySelector('a')!.getAttribute('href')).toBe('/initiatives/ABC-1');
		expect(items[0].querySelector('mark')!.textContent).toBe('NorthCloud');
		expect(items[1].querySelector('mark')).toBeNull();
	});

	it('says what can be searched', () => {
		app = mount(Page, { target: document.body });
		flushSync();
		expect(document.body.querySelector('label[for="search-q"]')!.textContent).toBe(
			'Title, brief, description, key or external reference'
		);
		expect(document.body.querySelector('input[type="search"]')!.getAttribute('placeholder')).toBe('e.g. 12, NorthCloud or JPD-30');
	});

	it('shows exact hits above every level, in the server order', async () => {
		net.mode.exactHits = true;
		app = mount(Page, { target: document.body });
		const items = await vi.waitFor(() => {
			flushSync();
			const li = [...document.body.querySelectorAll('li')];
			expect(li).toHaveLength(4);
			return li;
		});
		expect(items[0].querySelector('a')!.getAttribute('href')).toBe('/board?story=ABC-12');
		expect(items[0].textContent).toContain('matched key');
		expect(items[1].querySelector('a')!.getAttribute('href')).toBe('/board?story=ABC-9');
		expect(items[1].textContent).toContain('matched external reference');
		expect(items[2].querySelector('a')!.getAttribute('href')).toBe('/initiatives/ABC-1');
		expect([...document.body.querySelectorAll('h2')].map((h) => h.textContent?.trim())).toEqual([
			'Exact match 2',
			'Initiatives 1',
			'Stories 1'
		]);
	});

	it('keeps the typed text when Retry searches again', async () => {
		net.mode.fail = true;
		app = mount(Page, { target: document.body });
		const retry = await vi.waitFor(() => {
			flushSync();
			const b = [...document.body.querySelectorAll('button')].find((x) => x.textContent === 'Retry');
			expect(b).toBeDefined();
			return b!;
		});
		const input = document.body.querySelector('input[type="search"]') as HTMLInputElement;
		input.value = 'half typed';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		net.mode.fail = false;
		const before = net.asked.length;
		retry.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelectorAll('li')).toHaveLength(2);
		});
		expect(net.asked.length).toBeGreaterThan(before);
		expect(input.value).toBe('half typed');
	});

	it('submits the typed draft as a /search?q= navigation', async () => {
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelectorAll('li')).toHaveLength(2);
		});
		const form = document.body.querySelector('form[aria-label="Search items"]') as HTMLFormElement;
		const input = form.querySelector('input[type="search"]') as HTMLInputElement;
		input.value = 'a&b c';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		vi.mocked(goto).mockClear();
		form.requestSubmit();
		flushSync();
		expect(goto).toHaveBeenCalledTimes(1);
		expect(goto).toHaveBeenCalledWith(`/search?q=${encodeURIComponent('a&b c')}`);
	});
});
