// @vitest-environment jsdom
// The top navigation: Planning, Backlog, Board, Sprint, Decisions; the live indicator links to Activity.
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Story } from '#lib/api/types.ts';
import { session } from '#lib/me.svelte.ts';
import { discardDraft, openReport, report } from '#lib/report.svelte.ts';
import { me, story } from '#lib/test/fixtures.ts';
import Layout from './+layout.svelte';

const route = vi.hoisted(() => ({ path: '/epics', admin: false }));
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return new URL(`http://localhost${route.path}`);
		}
	}
}));
const router = vi.hoisted(() => ({ onNavigate: [] as ((n: unknown) => void)[] }));
vi.mock('$app/navigation', () => ({
	goto: vi.fn(),
	onNavigate: (fn: (n: unknown) => void) => {
		router.onNavigate.push(fn);
		return () => {};
	}
}));
// The dialog has its own tests: here a stand-in records the callbacks the layout hands it.
const dialog = vi.hoisted(() => ({
	props: null as null | { oncreated: (s: Story, epicTitle: string | null) => void; onviewstory: (key: string) => void }
}));
vi.mock('#lib/components/ReportDialog.svelte', () => ({
	default: (_anchor: unknown, props: NonNullable<typeof dialog.props>) => {
		dialog.props = props;
	}
}));
const net = vi.hoisted(() => ({ gets: [] as string[] }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	const { me, epic } = await import('#lib/test/fixtures.ts');
	installFetch(({ method, path }) => {
		net.gets.push(`${method} ${path}`);
		if (path === '/api/v1/me') return me();
		if (path === '/api/v1/epics/ABC-30') return epic({ key: 'ABC-30', title: 'Payments' });
		return failure('unexpected', 404);
	});
	return orig();
});
vi.mock('#lib/me.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/me.svelte.ts')>()),
	load: () => Promise.resolve(),
	isAdmin: () => route.admin
}));

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	route.admin = false;
	session.me = null;
	discardDraft();
	dialog.props = null;
	net.gets.length = 0;
	router.onNavigate.length = 0;
	sessionStorage.clear();
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
		expect(await labels()).toEqual(['Tokens', 'Activity', 'Users', 'Access', 'Sign out']);
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

describe('Report', () => {
	const button = () => [...document.body.querySelectorAll<HTMLButtonElement>('header button')].find((b) => /report/i.test(b.getAttribute('aria-label') ?? b.textContent ?? ''));
	const status = () => [...document.body.querySelectorAll('[role="status"]')].map((n) => n.textContent?.replace(/\s+/g, ' ').trim());
	const click = (el: Element | undefined) => {
		(el as HTMLElement).click();
		flushSync();
	};
	const button_named = (name: string) => [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === name);

	it('puts a Report button before the search form, reading Resume report while a draft exists', async () => {
		session.me = me();
		await open('/board');
		const b = button()!;
		expect(b.textContent).toContain('Report');
		expect(b.getAttribute('aria-label')).toBe('Report');
		expect(b.compareDocumentPosition(document.body.querySelector('form[role="search"]')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
		report.draft = null;
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.title = 'Checkout fails';
		report.open = false;
		flushSync();
		expect(button()!.getAttribute('aria-label')).toBe('Resume report');
	});

	it('prefills the epic chip on an epic page only, reading the epic once', async () => {
		session.me = me();
		await open('/epics/ABC-30');
		click(button());
		await vi.waitFor(() => expect(report.open).toBe(true));
		expect(report.draft!.epic).toEqual({ key: 'ABC-30', title: 'Payments' });
		expect(report.draft!.origin.path).toBe('/epics/ABC-30');
		expect(net.gets.filter((g) => g === 'GET /api/v1/epics/ABC-30')).toHaveLength(1);
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		discardDraft();
		net.gets.length = 0;
		await open('/board');
		click(button());
		await vi.waitFor(() => expect(report.open).toBe(true));
		expect(report.draft!.epic).toBeNull();
		expect(report.draft!.origin).toEqual({ path: '/board', label: 'Board' });
		expect(net.gets).not.toContain('GET /api/v1/epics/ABC-30');
	});

	it('keeps a notice after a create until another page is opened, and links the key', async () => {
		session.me = me();
		await open('/board');
		const bug = story({ key: 'ABC-612', kind: 'bug' });
		dialog.props!.oncreated(bug, null);
		flushSync();
		expect(status()).toContain(
			"Filed bug ABC-612. It's waiting for the team to place it; you won't get a notification, so check ABC-612 for updates. Report another Dismiss"
		);
		const a = document.body.querySelector<HTMLAnchorElement>('[role="status"] a')!;
		expect(a.textContent).toBe('ABC-612');
		expect(a.getAttribute('href')).toBe('/board?story=ABC-612');
		// Following the link changes only ?story=: the notice stays.
		const nav = (from: string, to: string) => router.onNavigate.forEach((f) => f({ from: { url: new URL(from) }, to: { url: new URL(to) } }));
		nav('http://localhost/board', 'http://localhost/board?story=ABC-612');
		flushSync();
		expect(status().join()).toContain('Filed bug ABC-612');
		nav('http://localhost/board?story=ABC-612', 'http://localhost/backlog');
		flushSync();
		expect(status().join()).not.toContain('Filed bug');
	});

	it('names the epic when filed under one, offers Report another and Dismiss', async () => {
		session.me = me();
		await open('/board');
		dialog.props!.oncreated(story({ key: 'ABC-612', kind: 'bug', epic: 'ABC-30' }), 'Payments');
		flushSync();
		expect(status()[0]).toMatch(/^Filed bug ABC-612 under ABC-30 Payments\./);
		click(button_named('Report another'));
		await vi.waitFor(() => expect(report.open).toBe(true));
		expect(report.draft!.title).toBe('');
		click(button_named('Dismiss'));
		expect(status().join()).not.toContain('Filed bug');
	});

	it('restores a draft saved for the signed-in email, once, and says so', async () => {
		report.draft = null;
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.title = 'Checkout fails';
		sessionStorage.setItem('reeve.report-draft', JSON.stringify({ email: me().email, draft: report.draft }));
		discardDraft({ keepStorage: true });
		session.me = me();
		await open('/board');
		expect((report.draft as { title: string } | null)?.title).toBe('Checkout fails');
		expect(button()!.getAttribute('aria-label')).toBe('Resume report');
		expect(status().join()).toContain('Your report draft was restored.');
		click(button_named('Dismiss'));
		expect(status().join()).not.toContain('Your report draft was restored.');
	});

	it('says nothing when the stored draft belongs to someone else', async () => {
		report.draft = null;
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.title = 'Checkout fails';
		sessionStorage.setItem('reeve.report-draft', JSON.stringify({ email: 'other@example.com', draft: report.draft }));
		discardDraft({ keepStorage: true });
		session.me = me();
		await open('/board');
		expect(report.draft).toBeNull();
		expect(status()).toEqual([]);
	});
});
