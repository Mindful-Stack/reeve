// @vitest-environment jsdom
// The board's Promise filter: it reads the server's promise state and period on each card (through `promisedMatches`),
// so a card whose promise does not match is hidden, and its select names the five options.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { page } from '$app/state';
import type { StoryPromise } from '#lib/api/types.ts';
import { withReadiness } from '#lib/test/fixtures.ts';
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
vi.mock('#lib/events.svelte.ts', () => ({
	live: { connected: false, lastId: 0, version: 0, latest: null, recent: [] },
	onLive: () => {},
	freshNote: () => null
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch((c: Call) => {
		if (c.method === 'GET' && c.path === '/api/v1/stories') return stories();
		if (c.method === 'GET' && c.path === '/api/v1/epics') return [];
		if (c.method === 'GET' && c.path === '/api/v1/sprints') return { active: null, draft: null, closed: [], today: '2026-11-12' };
		if (c.method === 'GET' && c.path === '/api/v1/decisions/queue') return { entries: [], summary: null };
		throw new Error(`unexpected ${c.method} ${c.path}`);
	});
	return orig();
});

const pr = (state: StoryPromise['state'], due: StoryPromise['due']): StoryPromise => ({
	own: { on: '2026-11-10', to: null, by: 'dana@example.com', at: '2026-11-01T09:00:00Z' },
	epic: null,
	on: '2026-11-10',
	holder: 'own',
	state,
	due,
	days: state === 'overdue' ? 2 : null
});
function stories() {
	return [
		withReadiness({ key: 'ABC-1', title: 'Overdue', delivery: 'in_progress', promise: pr('overdue', null) }),
		withReadiness({ key: 'ABC-2', title: 'Due this sprint', promise: pr('open', 'this_sprint') }),
		withReadiness({ key: 'ABC-3', title: 'No promise' })
	];
}

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	(page.url as URL).href = 'http://localhost/board';
});

const shownKeys = () => [...document.body.querySelectorAll<HTMLElement>('[data-testid^="card-"]')].map((c) => c.dataset.testid!.slice(5));
const promiseSelect = () =>
	[...document.body.querySelectorAll('label')].find((l) => l.firstChild?.textContent?.trim() === 'Promise')!.querySelector('select')!;

async function mounted(search = '') {
	(page.url as URL).href = `http://localhost/board${search}`;
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('[data-testid="card-ABC-1"]')).not.toBeNull();
	});
}

describe('board Promise filter', () => {
	it('?promised=overdue shows only the overdue card', async () => {
		await mounted('?promised=overdue');
		expect(shownKeys()).toEqual(['ABC-1']);
	});

	it('lists the five options and filters on a choice', async () => {
		await mounted();
		expect(shownKeys().sort()).toEqual(['ABC-1', 'ABC-2', 'ABC-3']);
		const select = promiseSelect();
		expect([...select.options].map((o) => [o.value, o.textContent])).toEqual([
			['', 'all stories'],
			['any', 'Promised (open or overdue)'],
			['this_sprint', 'Due this sprint, incl. overdue'],
			['next_sprint', 'Due next sprint'],
			['overdue', 'Overdue']
		]);
		select.value = 'this_sprint';
		select.dispatchEvent(new Event('change', { bubbles: true }));
		await vi.waitFor(() => {
			flushSync();
			expect(shownKeys().sort()).toEqual(['ABC-1', 'ABC-2']);
		});
		expect(String(page.url)).toBe('http://localhost/board?promised=this_sprint');
	});
});
