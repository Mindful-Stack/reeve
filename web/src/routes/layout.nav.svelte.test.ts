// @vitest-environment jsdom
// The top navigation: Planning, Backlog, Board, Sprint, Decisions; the live indicator links to Activity.
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { session } from '#lib/me.svelte.ts';
import { me } from '#lib/test/fixtures.ts';
import Layout from './+layout.svelte';

const route = vi.hoisted(() => ({ path: '/epics', admin: false }));
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return new URL(`http://localhost${route.path}`);
		}
	}
}));
vi.mock('$app/navigation', () => ({ goto: vi.fn(), onNavigate: () => () => {} }));
vi.mock('#lib/me.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/me.svelte.ts')>()),
	load: () => Promise.resolve(),
	isAdmin: () => route.admin
}));

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	route.admin = false;
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

async function open(path: string) {
	route.path = path;
	const children = createRawSnippet(() => ({ render: () => '<p>content</p>' }));
	app = mount(Layout, { target: document.body, props: { children } as never });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('nav')).not.toBeNull();
	});
}
const link = (name: string) => [...document.body.querySelectorAll('nav[aria-label="Main"] a')].find((a) => a.textContent === name) as HTMLAnchorElement | undefined;

describe('navigation', () => {
	it('lists the five destinations, Sprint between Board and Decisions, and no Epics link', async () => {
		await open('/backlog');
		expect([...document.body.querySelectorAll('nav[aria-label="Main"] a')].map((a) => a.textContent)).toEqual([
			'Planning', 'Backlog', 'Board', 'Sprint', 'Decisions'
		]);
		expect(link('Backlog')!.getAttribute('aria-current')).toBe('page');
		expect(link('Epics')).toBeUndefined();
	});

	it('marks Planning current only on /', async () => {
		await open('/board');
		expect(link('Planning')!.hasAttribute('aria-current')).toBe(false);
		expect(link('Board')!.getAttribute('aria-current')).toBe('page');
	});

	it('the live indicator links to activity and says offline, then live, for screen readers and the smoke', async () => {
		// Signed in, the layout opens the event stream; the fake opens only when the test says so.
		const streams: { onopen: (() => void) | null }[] = [];
		vi.stubGlobal(
			'EventSource',
			class {
				static CLOSED = 2;
				readyState = 0;
				onopen: (() => void) | null = null;
				onerror: (() => void) | null = null;
				constructor() {
					streams.push(this);
				}
				addEventListener() {}
				close() {}
			}
		);
		session.me = me();
		try {
			await open('/');
			const live = document.body.querySelector('a.live') as HTMLAnchorElement;
			const spoken = () => live.querySelector('.visually-hidden')?.textContent;
			expect(live.getAttribute('href')).toBe('/activity');
			expect(spoken()).toBe('offline');
			expect(streams).toHaveLength(1);
			streams[0].onopen!();
			flushSync();
			expect(spoken()).toBe('live');
			expect(live.classList).toContain('on');
		} finally {
			session.me = null;
			vi.unstubAllGlobals();
		}
	});

	it('/ focuses search unless typing in a field', async () => {
		await open('/');
		const search = document.body.querySelector('input[type="search"]') as HTMLInputElement;
		document.body.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
		expect(document.activeElement).toBe(search);
		search.blur();
		const other = document.createElement('textarea');
		document.body.append(other);
		other.focus();
		other.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
		expect(document.activeElement).toBe(other);
	});

	it('Ctrl+/ does not focus search', async () => {
		await open('/');
		document.body.dispatchEvent(new KeyboardEvent('keydown', { key: '/', ctrlKey: true, bubbles: true }));
		expect(document.activeElement).not.toBe(document.body.querySelector('input[type="search"]'));
	});

	it('/ inside an open dialog or a menu does not move focus', async () => {
		await open('/');
		const search = document.body.querySelector('input[type="search"]');
		const dialog = document.createElement('dialog');
		dialog.setAttribute('open', '');
		const inner = document.createElement('button');
		dialog.append(inner);
		const menu = document.createElement('div');
		menu.setAttribute('role', 'menu');
		const item = document.createElement('button');
		menu.append(item);
		document.body.append(dialog, menu);
		for (const el of [inner, item]) {
			el.focus();
			el.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true }));
			expect(document.activeElement).toBe(el);
			expect(document.activeElement).not.toBe(search);
		}
	});

	it('/ is not prevented when there is no search input', async () => {
		route.path = '/auth/signin'; // signed out: no top bar, so no search input
		const children = createRawSnippet(() => ({ render: () => '<p>content</p>' }));
		app = mount(Layout, { target: document.body, props: { children } as never });
		flushSync();
		expect(document.body.querySelector('input[type="search"]')).toBeNull();
		const ev = new KeyboardEvent('keydown', { key: '/', bubbles: true, cancelable: true });
		document.body.dispatchEvent(ev);
		expect(ev.defaultPrevented).toBe(false);
	});
});

describe('account menu', () => {
	const labels = async () => {
		const trigger = [...document.body.querySelectorAll('button')].find((b) => /^Account:/.test(b.getAttribute('aria-label') ?? '')) as HTMLButtonElement;
		trigger.click();
		flushSync();
		return [...document.body.querySelectorAll('[role="menuitem"]')].map((i) => i.textContent);
	};

	it('lists Tokens, Activity and Sign out for a member', async () => {
		await open('/');
		expect(await labels()).toEqual(['Tokens', 'Activity', 'Sign out']);
	});

	it('adds Users for an admin', async () => {
		route.admin = true;
		await open('/');
		expect(await labels()).toEqual(['Tokens', 'Activity', 'Users', 'Sign out']);
	});
});

describe('top bar search field', () => {
	it('shows the query of /search so a reload keeps it', async () => {
		await open('/search?q=northcloud');
		expect((document.body.querySelector('input[type="search"]') as HTMLInputElement).value).toBe('northcloud');
	});
	it('trims the query of /search as the page does', async () => {
		await open('/search?q=%20%20northcloud%20');
		expect((document.body.querySelector('input[type="search"]') as HTMLInputElement).value).toBe('northcloud');
	});
});
