// @vitest-environment jsdom
// The layout owns the drawers' lifecycle: `?story=KEY` mounts the story drawer (keyed by KEY) and `?decision=ID` the
// decision drawer, Escape drops the parameter in place, and focus returns to whatever opened the first drawer, across
// drawer-to-drawer navigation (a story's link from a decision, and back).
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { goto } from '$app/navigation';
import { page } from '$app/state';
import { resetDecisionDraftsForTests } from '#lib/decisionDrafts.svelte.ts';
import Layout from './+layout.svelte';

const net = vi.hoisted(() => ({ gets: [] as string[] }));

// The URL is reactive (a SvelteURL), so a goto re-renders the layout as in the browser. `navigate` follows SvelteKit's
// client-side order for a link click: the onNavigate callbacks, then the blur of the clicked link (the default focus
// reset), then the new URL.
vi.mock('$app/state', async () => {
	const { SvelteURL } = await import('svelte/reactivity');
	return { page: { url: new SvelteURL('http://localhost/board') } };
});
const router = vi.hoisted(() => ({ onNavigate: [] as ((n: unknown) => void)[] }));
vi.mock('$app/navigation', () => ({
	goto: vi.fn(async (url: URL) => {
		((await import('$app/state')).page.url as URL).href = url.href;
	}),
	onNavigate: (fn: (n: unknown) => void) => {
		router.onNavigate.push(fn);
		return () => router.onNavigate.splice(router.onNavigate.indexOf(fn), 1);
	}
}));
vi.mock('#lib/me.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/me.svelte.ts')>()),
	session: { me: { email: 'me@example.com', role: 'member', token_kind: 'interactive' }, config: null },
	load: () => Promise.resolve(),
	isAdmin: () => false
}));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	connect: () => () => {}
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const real = await orig<typeof import('#lib/api/client.ts')>();
	const ok = (value: unknown) => Promise.resolve({ data: { outcome: 'ok', value }, response: { status: 200 } as Response });
	const { blockedStory, decision, queueEntry, story } = await import('#lib/test/fixtures.ts');
	return {
		...real,
		api: {
			GET: (path: string, init?: { params?: { path?: { key?: string; id?: number } } }) => {
				const key = init?.params?.path?.key;
				const id = init?.params?.path?.id;
				net.gets.push(key ? path.replace('{key}', key) : id ? path.replace('{id}', String(id)) : path);
				if (path === '/api/v1/decisions/{id}/context')
					return ok(
						queueEntry({
							decision: decision({ id: id!, title: `Decision ${id}`, on: 'RV-1' }),
							lane: 'this_sprint',
							needed_in: 'this_sprint',
							blocks: [blockedStory({ key: 'RV-5', title: 'Held story', needed_in: 'this_sprint' })]
						})
					);
				if (path === '/api/v1/stories/{key}') return ok(story({ key: key!, title: `Story ${key}` }));
				if (path === '/api/v1/stories/{key}/ready') return ok({ ready: true, reasons: [] });
				return ok([]);
			}
		}
	};
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.gets.length = 0;
	vi.mocked(goto).mockClear();
	resetDecisionDraftsForTests();
	(page.url as URL).href = 'http://localhost/board';
});

/** A link click as the router runs it: onNavigate, the focus reset's blur, then the new URL. */
function navigate(href: string) {
	const from = new URL((page.url as URL).href);
	const to = new URL(href);
	for (const fn of router.onNavigate) fn({ from: { url: from }, to: { url: to }, type: 'link' });
	(document.activeElement as HTMLElement | null)?.blur();
	(page.url as URL).href = href;
}

const drawer = (key: string) => document.body.querySelector<HTMLElement>(`[aria-label="Story ${key}"]`);

async function drawerFocused(key: string) {
	await vi.waitFor(() => {
		flushSync();
		expect(drawer(key)).not.toBeNull();
		expect(document.activeElement).toBe(drawer(key)!.querySelector('h2'));
	});
}

describe('the layout and the story drawer', () => {
	it('opens on ?story=, closes on Escape in place, and returns focus to the first opener across drawers', async () => {
		const opener = document.createElement('button');
		opener.id = 'opener';
		document.body.append(opener);
		opener.focus();
		const children = createRawSnippet(() => ({ render: () => '<p>content</p>' }));
		app = mount(Layout, { target: document.body, props: { children } as never });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('main')).not.toBeNull();
		});

		// 1. Following the opener's link: the drawer appears and its heading takes focus.
		navigate('http://localhost/board?story=RV-5');
		await drawerFocused('RV-5');

		// 2. Escape drops the parameter in place; the drawer goes and focus returns to the opener.
		document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		expect(goto).toHaveBeenCalledTimes(1);
		const [to, opts] = vi.mocked(goto).mock.calls[0] as [URL, unknown];
		expect(new URL(to).searchParams.has('story')).toBe(false);
		expect(new URL(to).pathname).toBe('/board');
		expect(opts).toEqual({ replace: true, reset: false });
		await vi.waitFor(() => {
			flushSync();
			expect(drawer('RV-5')).toBeNull();
			expect(document.activeElement).toBe(opener);
		});

		// 3. Open again from the opener, then go to another story: the drawer remounts and loads it, and closing still
		// returns focus to the first opener.
		opener.focus();
		navigate('http://localhost/board?story=RV-5');
		await drawerFocused('RV-5');
		navigate('http://localhost/board?story=RV-6');
		await drawerFocused('RV-6');
		expect(drawer('RV-5')).toBeNull();
		expect(net.gets).toContain('/api/v1/stories/RV-6');
		document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		await vi.waitFor(() => {
			flushSync();
			expect(drawer('RV-6')).toBeNull();
			expect(document.activeElement).toBe(opener);
		});
	});

	it('falls back to the header Report button when the recorded opener is gone (a hit viewed from the dialog)', async () => {
		const hit = document.createElement('button');
		document.body.append(hit);
		hit.focus();
		const children = createRawSnippet(() => ({ render: () => '<p>content</p>' }));
		app = mount(Layout, { target: document.body, props: { children } as never });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('main')).not.toBeNull();
		});
		navigate('http://localhost/board?story=RV-5');
		await drawerFocused('RV-5');
		hit.remove(); // the dialog, and its hit button with it, unmounted
		document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		await vi.waitFor(() => {
			flushSync();
			expect(drawer('RV-5')).toBeNull();
			expect(document.activeElement).toBe(document.body.querySelector('button.report'));
		});
	});
});

const decisionDrawer = (id: number) => document.body.querySelector<HTMLElement>(`[data-testid="decision-drawer"][aria-label="Decision #${id}"]`);

async function decisionFocused(id: number) {
	await vi.waitFor(() => {
		flushSync();
		expect(decisionDrawer(id)).not.toBeNull();
		expect(document.activeElement).toBe(decisionDrawer(id)!.querySelector('h2'));
	});
}

describe('the layout and the decision drawer', () => {
	async function mountLayout() {
		const children = createRawSnippet(() => ({ render: () => '<p>content</p>' }));
		app = mount(Layout, { target: document.body, props: { children } as never });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('main')).not.toBeNull();
		});
	}

	it('opens on ?decision=, swaps to the story drawer from a held story, comes back on Back, and closes on Escape to the opener', async () => {
		const opener = document.createElement('button');
		document.body.append(opener);
		opener.focus();
		await mountLayout();

		// 1. Following the opener's link mounts the decision drawer, which reads the decision's context.
		const decisionUrl = 'http://localhost/board?decision=7';
		navigate(decisionUrl);
		await decisionFocused(7);
		expect(net.gets).toContain('/api/v1/decisions/7/context');
		expect(drawer('RV-5')).toBeNull();

		// 2. A held story's link swaps to the story drawer: the URL has story and no decision.
		const link = await vi.waitFor(() => {
			flushSync();
			const a = decisionDrawer(7)!.querySelector<HTMLAnchorElement>('.held a.key');
			expect(a).not.toBeNull();
			return a!;
		});
		const href = link.getAttribute('href')!;
		expect(new URL(href, 'http://localhost').searchParams.get('story')).toBe('RV-5');
		expect(new URL(href, 'http://localhost').searchParams.has('decision')).toBe(false);
		navigate(new URL(href, 'http://localhost').href);
		await drawerFocused('RV-5');
		expect(decisionDrawer(7)).toBeNull();

		// 3. Back shows the decision drawer again.
		navigate(decisionUrl);
		await decisionFocused(7);
		expect(drawer('RV-5')).toBeNull();

		// 4. Escape drops the parameter in place and focus returns to the opener of the first drawer.
		document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		expect(goto).toHaveBeenCalledTimes(1);
		const [to, opts] = vi.mocked(goto).mock.calls[0] as [URL, unknown];
		expect(new URL(to).searchParams.has('decision')).toBe(false);
		expect(new URL(to).pathname).toBe('/board');
		expect(opts).toEqual({ replace: true, reset: false });
		await vi.waitFor(() => {
			flushSync();
			expect(decisionDrawer(7)).toBeNull();
			expect(document.activeElement).toBe(opener);
		});
	});

	it('closing drops both parameters, and a decision that is not a positive whole number opens nothing', async () => {
		await mountLayout();
		navigate('http://localhost/board?decision=abc');
		flushSync();
		expect(document.body.querySelector('[data-testid="decision-drawer"]')).toBeNull();
		navigate('http://localhost/board?decision=0');
		flushSync();
		expect(document.body.querySelector('[data-testid="decision-drawer"]')).toBeNull();

		navigate('http://localhost/board?decision=7');
		await decisionFocused(7);
		document.activeElement!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		const [to] = vi.mocked(goto).mock.calls[0] as [URL];
		expect([...new URL(to).searchParams.keys()]).toEqual([]);
	});
});
