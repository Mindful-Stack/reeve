// @vitest-environment jsdom
// Spec §11: after a move made from a card's ⋯ menu, focus lands on that card's ⋯ trigger in its new column.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { page } from '$app/state';
import type { Story } from '#lib/api/types.ts';
import Page from './+page.svelte';

const KEY = 'ABC-1';
const server = vi.hoisted(() => ({ delivery: 'todo' as Story['delivery'], ready: true, moves: [] as unknown[] }));
// The URL is reactive (a SvelteURL), so a goto (the board's filters, ?notready=1) re-renders the page as in the browser.
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
	const real = await orig<typeof import('#lib/api/client.ts')>();
	const ok = (value: unknown) => Promise.resolve({ data: { outcome: 'ok', value }, response: { status: 200 } as Response });
	const { story: storyDto, withReadiness } = await import('#lib/test/fixtures.ts');
	const story = (): Story =>
		storyDto({ key: 'ABC-1', title: 'The story', rank: 1, delivery: server.delivery, version: server.moves.length + 1 });
	return {
		...real,
		api: {
			GET: (path: string) =>
				path === '/api/v1/stories' ? ok([withReadiness(story(), server.ready ? [] : [{ reason: 'no_criteria' }])]) : ok([]),
			POST: (path: string, init: { body: { to: Story['delivery'] } }) => {
				if (path !== '/api/v1/stories/{key}/move') throw new Error(`unexpected POST ${path}`);
				server.moves.push(init.body);
				server.delivery = init.body.to;
				return ok(story());
			}
		}
	};
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	server.delivery = 'todo';
	server.ready = true;
	server.moves = [];
	(page.url as URL).href = 'http://localhost/board';
});

const trigger = (col: string) =>
	document.body.querySelector<HTMLButtonElement>(`[data-testid="column-${col}"] button[aria-label="Actions for ${KEY}"]`);

describe('board refocus', () => {
	it('returns focus to the moved card’s ⋯ trigger in its new column', async () => {
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(trigger('todo')).not.toBeNull();
		});

		trigger('todo')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
		});
		const item = [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
			(b) => b.textContent === 'Move to in progress'
		);
		item!.click();

		await vi.waitFor(() => {
			flushSync();
			expect(trigger('in_progress')).not.toBeNull();
			expect(document.activeElement).toBe(trigger('in_progress'));
		});
		expect(server.moves).toEqual([expect.objectContaining({ to: 'in_progress' })]);
		expect(trigger('todo')).toBeNull();
	});

	it('opens Not ready when a not-ready story moves back to todo, then focuses its ⋯ trigger there', async () => {
		server.delivery = 'in_progress';
		server.ready = false;
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(trigger('in_progress')).not.toBeNull();
		});

		trigger('in_progress')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
		});
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
			.find((b) => b.textContent === 'Move to todo')!
			.click();

		await vi.waitFor(() => {
			flushSync();
			const t = trigger('todo');
			expect(t?.closest('details.not-ready')?.hasAttribute('open')).toBe(true);
			expect(document.activeElement).toBe(t);
		});
		expect(page.url.searchParams.get('notready')).toBe('1');
	});

	it('Move to done asks for the resolution in a Dialog, then posts it', async () => {
		server.delivery = 'in_progress';
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(trigger('in_progress')).not.toBeNull();
		});
		trigger('in_progress')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
		});
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')]
			.find((b) => b.textContent === 'Move to done')!
			.click();
		flushSync();
		expect(document.body.querySelector('dialog h2')?.textContent).toBe(`Move ${KEY} to done`);
		[...document.body.querySelectorAll<HTMLButtonElement>('dialog button')].find((b) => b.textContent === 'completed')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(trigger('done')).not.toBeNull();
		});
		expect(document.body.querySelector('dialog')).toBeNull();
		expect(server.moves).toEqual([expect.objectContaining({ to: 'done', resolution: 'completed' })]);
	});
});
