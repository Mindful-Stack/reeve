// @vitest-environment jsdom
// The board's Owner select: the people offered are the owners of the loaded cards, and the choice lives in
// ?mine=1 or ?owner=.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { page } from '$app/state';
import type { StoryWithReadiness } from '#lib/api/types.ts';
import { session } from '#lib/me.svelte.ts';
import { me, withReadiness } from '#lib/test/fixtures.ts';
import type { Call } from '#lib/test/net.ts';
import Page from './+page.svelte';

vi.mock('$app/state', async () => {
	const { SvelteURL } = await import('svelte/reactivity');
	return { page: { url: new SvelteURL('http://localhost/board') } };
});
vi.mock('$app/navigation', () => ({
	goto: vi.fn(async (url: URL) => {
		((await import('$app/state')).page.url as URL).href = url.href;
	})
}));
vi.mock('#lib/me.svelte.ts', async () => ({
	session: await (async () => {
		const { config, me } = await import('#lib/test/fixtures.ts');
		return { me: me({ email: 'me@example.com' }), config: config({ done_window_days: 14 }) };
	})(),
	isInteractive: () => true,
	isAdmin: () => false,
	load: () => Promise.resolve()
}));
const live = vi.hoisted(() => ({ fns: [] as (() => void)[] }));
vi.mock('#lib/events.svelte.ts', () => ({
	live: { connected: false, lastId: 0, version: 0, latest: null, recent: [] },
	onLive: (fn: () => void) => live.fns.push(fn),
	freshNote: () => null
}));
const net = vi.hoisted(() => ({ handler: (_c: import('#lib/test/net.ts').Call): unknown => null }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch((c) => net.handler(c));
	return orig();
});

let stories: StoryWithReadiness[];

function serve(c: Call): unknown {
	const { method, path } = c;
	if (method === 'GET' && path === '/api/v1/stories') return stories;
	if (method === 'GET' && path === '/api/v1/epics') return [];
	// No active sprint: the Goal select is absent, so the Owner select is the only select named by test id.
	if (method === 'GET' && path === '/api/v1/sprints') return { active: null, closed: [], draft: null, today: '2026-11-12' };
	throw new Error(`unexpected ${method} ${path}`);
}

beforeEach(() => {
	stories = [
		withReadiness({ key: 'ABC-1', owner: 'me@example.com' }),
		withReadiness({ key: 'ABC-2', owner: 'bo@example.com', delivery: 'in_progress' }),
		withReadiness({ key: 'ABC-3', owner: 'ana@example.com', kind: 'bug' }),
		withReadiness({ key: 'ABC-4' }),
		withReadiness({ key: 'ABC-5', owner: 'ana@example.com', delivery: 'testing' })
	];
	net.handler = serve;
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	live.fns.length = 0;
	(page.url as URL).href = 'http://localhost/board';
	(session as { me: unknown }).me = me({ email: 'me@example.com' });
});

const q = <T extends Element = HTMLElement>(sel: string) => document.body.querySelector<T>(sel);
const shownKeys = () =>
	[...document.body.querySelectorAll<HTMLElement>('[data-testid^="card-"]')].map((c) => c.dataset.testid!.slice(5));
const ownerSelect = () => q<HTMLSelectElement>('[data-testid="owner-filter"]')!;
const options = () => [...ownerSelect().querySelectorAll('option')].map((o) => [o.value, o.textContent]);
const ALL = ['ABC-1', 'ABC-2', 'ABC-3', 'ABC-4', 'ABC-5'];
/** Picks an option as a person does. Svelte delegates `change`, so the event must bubble. */
function choose(select: HTMLSelectElement, value: string) {
	select.value = value;
	select.dispatchEvent(new Event('change', { bubbles: true }));
}
async function shows(keys: string[]) {
	await vi.waitFor(() => {
		flushSync();
		expect(shownKeys().sort()).toEqual(keys);
	});
}
async function mounted(keys: string[] = ALL) {
	app = mount(Page, { target: document.body });
	await shows(keys);
}

describe('board owner select', () => {
	it('offers all stories, Me and Unowned, then the other owners on the board in a group', async () => {
		await mounted();
		expect(options()).toEqual([
			['', 'all stories'],
			['me', 'Me'],
			['none', 'Unowned'],
			['ana@example.com', 'ana'],
			['bo@example.com', 'bo']
		]);
		const group = ownerSelect().querySelector('optgroup')!;
		expect(group.label).toBe('On this board');
		expect([...group.querySelectorAll('option')].map((o) => o.value)).toEqual(['ana@example.com', 'bo@example.com']);
		expect(ownerSelect().lastElementChild).toBe(group);
		expect(ownerSelect().value).toBe('');
		expect(ownerSelect().closest('label')?.textContent).toContain('Owner');
		// The toggle it replaces is gone.
		expect(q('[aria-label="Whose"]')).toBeNull();
		expect(document.body.textContent).not.toContain('Whose');
	});

	it('signed out there is no Me, and every owner is in the group', async () => {
		(session as { me: unknown }).me = null;
		await mounted();
		expect(options().map(([v]) => v)).toEqual(['', 'none', 'ana@example.com', 'bo@example.com', 'me@example.com']);
	});

	it('has no group when nobody else owns a card', async () => {
		stories = [withReadiness({ key: 'ABC-1', owner: 'me@example.com' }), withReadiness({ key: 'ABC-4' })];
		await mounted(['ABC-1', 'ABC-4']);
		expect(ownerSelect().querySelector('optgroup')).toBeNull();
		expect(options().map(([v]) => v)).toEqual(['', 'me', 'none']);
	});

	it('Me, a person and Unowned each write one parameter, drop the other and show the matching cards', async () => {
		await mounted();
		choose(ownerSelect(), 'me');
		await shows(['ABC-1']);
		expect(page.url.search).toBe('?mine=1');

		choose(ownerSelect(), 'ana@example.com');
		await shows(['ABC-3', 'ABC-5']);
		expect(page.url.searchParams.get('owner')).toBe('ana@example.com');
		expect(page.url.searchParams.has('mine')).toBe(false);
		// A person's cards stay in their own columns: the select filters, it never moves cards.
		expect(q('[data-testid="column-testing"] [data-testid="card-ABC-5"]')).not.toBeNull();

		choose(ownerSelect(), 'none');
		await shows(['ABC-4']);
		expect(page.url.searchParams.get('owner')).toBe('none');

		choose(ownerSelect(), 'me');
		await shows(['ABC-1']);
		expect(page.url.searchParams.has('owner')).toBe(false);

		choose(ownerSelect(), '');
		await shows(ALL);
		expect(page.url.search).toBe('');
	});

	it('opens on ?mine=1, and on ?owner=<my email>, with Me selected', async () => {
		(page.url as URL).href = 'http://localhost/board?mine=1';
		await mounted(['ABC-1']);
		expect(ownerSelect().value).toBe('me');
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		(page.url as URL).href = 'http://localhost/board?owner=ME@example.com';
		await mounted(['ABC-1']);
		expect(ownerSelect().value).toBe('me');
	});

	it('an owner in the URL spelled in another case selects that person', async () => {
		(page.url as URL).href = 'http://localhost/board?owner=ANA@example.com';
		await mounted(['ABC-3', 'ABC-5']);
		expect(ownerSelect().value).toBe('ana@example.com');
	});

	it('an owner with no card on the board is still applied, shown and selected', async () => {
		(page.url as URL).href = 'http://localhost/board?owner=cy@example.com';
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(ownerSelect().value).toBe('cy@example.com');
			// The columns have loaded (their counts show) and hold no card.
			expect(q('[data-testid="column-todo"] .count')?.textContent).toBe('0');
		});
		expect(shownKeys()).toEqual([]);
		expect(options().map(([v]) => v)).toContain('cy@example.com');
	});

	it('with both ?mine=1 and ?owner=, the owner is the one applied', async () => {
		(page.url as URL).href = 'http://localhost/board?mine=1&owner=bo@example.com';
		await mounted(['ABC-2']);
		expect(ownerSelect().value).toBe('bo@example.com');
	});

	it('a live refetch that brings a card owned by someone new offers them and keeps the selection', async () => {
		(page.url as URL).href = 'http://localhost/board?owner=ana@example.com';
		await mounted(['ABC-3', 'ABC-5']);
		stories = [...stories, withReadiness({ key: 'ABC-6', owner: 'dee@example.com' })];
		for (const f of live.fns) f();
		await vi.waitFor(() => {
			flushSync();
			expect(options().map(([v]) => v)).toContain('dee@example.com');
		});
		expect(ownerSelect().value).toBe('ana@example.com');
		expect(shownKeys().sort()).toEqual(['ABC-3', 'ABC-5']);
	});
});
