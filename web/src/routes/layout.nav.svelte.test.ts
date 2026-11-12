// @vitest-environment jsdom
// The top navigation: Planning, Backlog, Board, Sprint, Decisions; the live indicator links to Activity.
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Story } from '#lib/api/types.ts';
import { session } from '#lib/me.svelte.ts';
import { discardDraft, openReport, report } from '#lib/report.svelte.ts';
import { me, story } from '#lib/test/fixtures.ts';
import { settle } from '#lib/test/net.ts';
import { goto } from '$app/navigation';
import Layout from './+layout.svelte';

// The URL is reactive (a SvelteURL), as in the browser: setting `route.path` is a navigation the layout sees.
const route = vi.hoisted(() => ({
	url: null as URL | null,
	admin: false,
	get path() {
		return this.url!.pathname + this.url!.search;
	},
	set path(p: string) {
		this.url!.href = `http://localhost${p}`;
	}
}));
vi.mock('$app/state', async () => {
	const { SvelteURL } = await import('svelte/reactivity');
	route.url = new SvelteURL('http://localhost/epics');
	return { page: { url: route.url } };
});
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
const net = vi.hoisted(() => ({
	gets: [] as string[],
	hold: null as null | Promise<void>,
	searchHold: null as null | Promise<void>,
	untriaged: 0 as number | null, // the list's length, or null for a failing read
	untriagedHold: null as null | Promise<void>
}));
const live = vi.hoisted(() => ({ fns: [] as (() => unknown)[] }));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => {
		live.fns.push(fn);
	}
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	const { me, epic, withReadiness } = await import('#lib/test/fixtures.ts');
	installFetch(({ method, path, url }) => {
		net.gets.push(`${method} ${path}`);
		if (path === '/api/v1/me') return me();
		if (path === '/api/v1/search') return Promise.resolve(net.searchHold).then(() => [{ key: 'ABC-12', item: 'story', kind: 'story', title: 'Twelve', matched: ['key'], exact: true }]);
		if (path === '/api/v1/epics/ABC-30') return Promise.resolve(net.hold).then(() => epic({ key: 'ABC-30', title: 'Payments' }));
		if (path === '/api/v1/stories' && url.search === '?untriaged=true') {
			const n = net.untriaged; // what the server held when asked
			return Promise.resolve(net.untriagedHold).then(() =>
				n === null ? failure('down') : Array.from({ length: n }, (_, i) => withReadiness({ key: `ABC-${i + 1}` }))
			);
		}
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
// A signed-in layout opens the event stream, and jsdom has no EventSource: an inert one for every test.
beforeEach(() => {
	vi.stubGlobal(
		'EventSource',
		class {
			static CLOSED = 2;
			readyState = 0;
			onopen: (() => void) | null = null;
			onerror: (() => void) | null = null;
			addEventListener() {}
			close() {}
		}
	);
});
afterEach(() => {
	vi.unstubAllGlobals();
	route.admin = false;
	session.me = null;
	discardDraft();
	dialog.props = null;
	net.gets.length = 0;
	net.hold = null;
	net.searchHold = null;
	net.untriaged = 0;
	net.untriagedHold = null;
	live.fns.length = 0;
	router.onNavigate.length = 0;
	sessionStorage.clear();
	document.documentElement.style.removeProperty('--top-bar-height');
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
	it('lists the six destinations, Home first, Sprint between Board and Decisions, and no Epics link', async () => {
		await open('/backlog');
		expect([...document.body.querySelectorAll('nav[aria-label="Main"] a')].map((a) => a.textContent)).toEqual([
			'Home', 'Planning', 'Backlog', 'Board', 'Sprint', 'Decisions'
		]);
		expect(link('Backlog')!.getAttribute('aria-current')).toBe('page');
		expect(link('Epics')).toBeUndefined();
	});

	it('marks Home current only on / and Planning on /planning', async () => {
		await open('/');
		expect(link('Home')!.getAttribute('aria-current')).toBe('page');
		expect(link('Planning')!.hasAttribute('aria-current')).toBe(false);
		unmount(app!);
		await open('/planning');
		expect(link('Home')!.hasAttribute('aria-current')).toBe(false);
		expect(link('Planning')!.getAttribute('aria-current')).toBe('page');
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
	it("shows the backlog's text filter", async () => {
		await open('/backlog?q=zebra&state=open');
		expect((document.body.querySelector('input[type="search"]') as HTMLInputElement).value).toBe('zebra');
	});
	const submit = (text: string) => {
		const input = document.body.querySelector('input[type="search"]') as HTMLInputElement;
		input.value = text;
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		(input.form as HTMLFormElement).requestSubmit();
	};
	it('text filters the open backlog by relevance, without a lookup', async () => {
		vi.mocked(goto).mockClear();
		await open('/board');
		submit('zebra crossing');
		await vi.waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/backlog?q=zebra+crossing&state=open&view=relevance'));
		expect(net.gets.filter((g) => g.includes('/search'))).toEqual([]);
	});
	it('a key opens its story over the current page; the form is busy while the lookup is pending', async () => {
		vi.mocked(goto).mockClear();
		await open('/board?mine=1');
		submit('12');
		flushSync();
		expect(document.body.querySelector('form[role="search"]')!.getAttribute('aria-busy')).toBe('true');
		await vi.waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/board?mine=1&story=ABC-12', { reset: false }));
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('form[role="search"]')!.getAttribute('aria-busy')).toBe('false');
		});
	});
	it('Escape leaves the field', async () => {
		await open('/board');
		const input = document.body.querySelector('input[type="search"]') as HTMLInputElement;
		input.focus();
		input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		expect(document.activeElement).not.toBe(input);
	});
	it('Escape while a key lookup is pending cancels it: the form is no longer busy and the answer opens nothing', async () => {
		vi.mocked(goto).mockClear();
		let release: () => void = () => {};
		net.searchHold = new Promise((r) => (release = r));
		await open('/board');
		submit('ABC-12');
		await vi.waitFor(() => expect(net.gets).toContain('GET /api/v1/search'));
		flushSync();
		expect(document.body.querySelector('form[role="search"]')!.getAttribute('aria-busy')).toBe('true');
		const input = document.body.querySelector('input[type="search"]') as HTMLInputElement;
		input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
		flushSync();
		expect(document.body.querySelector('form[role="search"]')!.getAttribute('aria-busy')).toBe('false');
		release();
		await new Promise((r) => setTimeout(r, 20));
		flushSync();
		expect(vi.mocked(goto)).not.toHaveBeenCalled();
		expect(document.body.querySelector('form[role="search"]')!.getAttribute('aria-busy')).toBe('false');
	});
	it("Escape restores the page's text instead of clearing the field", async () => {
		await open('/backlog?q=zebra');
		const input = document.body.querySelector('input[type="search"]') as HTMLInputElement;
		input.focus();
		input.value = 'lio';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		const esc = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
		input.dispatchEvent(esc);
		flushSync();
		expect(esc.defaultPrevented).toBe(true);
		expect(input.value).toBe('zebra');
		expect(document.activeElement).not.toBe(input);
	});
	it('the placeholder says a key jumps', async () => {
		await open('/board');
		expect(document.body.querySelector('input[type="search"]')!.getAttribute('placeholder')).toBe('Search or jump to a key');
	});
	it('text typed on the backlog survives a filter change and the drawer; a new text in the URL replaces it', async () => {
		await open('/backlog?q=zebra');
		const input = document.body.querySelector('input[type="search"]') as HTMLInputElement;
		input.value = 'lion';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		route.path = '/backlog?q=zebra&kind=bug';
		flushSync();
		expect(input.value).toBe('lion');
		route.path = '/backlog?q=zebra&kind=bug&story=S-1';
		flushSync();
		expect(input.value).toBe('lion');
		route.path = '/backlog?q=tiger';
		flushSync();
		expect(input.value).toBe('tiger');
	});
	it('text on a filtered backlog keeps its filters, sorts by relevance and edits the URL in place', async () => {
		vi.mocked(goto).mockClear();
		await open('/backlog?kind=bug');
		submit('zebra');
		await vi.waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/backlog?kind=bug&q=zebra&view=relevance', { replace: true, reset: false }));
	});
	it("after a key jump the field shows the page's own text again, not the key", async () => {
		vi.mocked(goto).mockClear();
		await open('/backlog?q=zebra');
		submit('12');
		await vi.waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalledWith('/backlog?q=zebra&story=ABC-12', { reset: false }));
		await vi.waitFor(() => {
			flushSync();
			expect((document.body.querySelector('input[type="search"]') as HTMLInputElement).value).toBe('zebra');
		});
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

	it('started with a drawer open, keeps the page and the story in the origin, and prefills the epic only on an epic page', async () => {
		session.me = me();
		await open('/board?story=ABC-1');
		click(button());
		await vi.waitFor(() => expect(report.open).toBe(true));
		expect(report.draft!.origin.path).toBe('/board?story=ABC-1');
		expect(report.draft!.epic).toBeNull(); // the story never prefills the chip
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		discardDraft();
		await open('/epics/ABC-30?story=ABC-1');
		click(button());
		await vi.waitFor(() => expect(report.open).toBe(true));
		expect(report.draft!.epic).toEqual({ key: 'ABC-30', title: 'Payments' }); // the page's epic, not the story's
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

describe('Report review fixes', () => {
	const btn = () => document.body.querySelector<HTMLButtonElement>('button.report')!;
	const status = () => [...document.body.querySelectorAll('[role="status"]')].map((n) => n.textContent ?? '').join('|');
	const nav = (from: string, to: string) => router.onNavigate.forEach((f) => f({ from: { url: new URL(from) }, to: { url: new URL(to) } }));
	const saveDraft = () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.title = 'Checkout fails';
		sessionStorage.setItem('reeve.report-draft', JSON.stringify({ email: me().email, draft: report.draft }));
		discardDraft({ keepStorage: true });
	};

	it('does not open when the page changed while the epic was read, and sends one GET for a double click', async () => {
		session.me = me();
		let release!: () => void;
		net.hold = new Promise((r) => (release = r));
		await open('/epics/ABC-30');
		btn().click();
		flushSync();
		btn().click();
		flushSync();
		expect(btn().getAttribute('aria-busy')).toBe('true');
		route.path = '/board';
		release();
		await vi.waitFor(() => expect(btn().getAttribute('aria-busy')).toBe('false'));
		expect(net.gets.filter((g) => g === 'GET /api/v1/epics/ABC-30')).toHaveLength(1);
		expect(report.open).toBe(false);
	});

	it('opens on a malformed %-sequence in the epic key without throwing', async () => {
		session.me = me();
		await open('/epics/ABC-%E0%A4%A');
		btn().click();
		await vi.waitFor(() => expect(report.open).toBe(true));
		expect(report.draft!.origin.label).toBe('ABC-%E0%A4%A');
	});

	it('the restored line goes when a report is filed, the draft is discarded or the page changes, and never returns', async () => {
		saveDraft();
		session.me = me();
		await open('/board');
		expect(status()).toContain('Your report draft was restored.');
		dialog.props!.oncreated(story({ key: 'ABC-612', kind: 'bug' }), null);
		flushSync();
		expect(status()).not.toContain('restored');
		expect(status()).toContain('Filed bug');
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		session.me = null;
		discardDraft();
		saveDraft();
		session.me = me();
		await open('/board');
		expect(status()).toContain('restored');
		discardDraft();
		flushSync();
		expect(status()).not.toContain('restored');
		// Once: a later change of the draft does not bring the line back.
		saveDraft();
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.title = 'Another';
		flushSync();
		expect(status()).not.toContain('restored');
		discardDraft();
	});

	it('the restored line clears on navigation', async () => {
		saveDraft();
		session.me = me();
		await open('/board');
		nav('http://localhost/board', 'http://localhost/backlog');
		flushSync();
		expect(status()).not.toContain('restored');
	});

	it('closing the drawer keeps the filed notice', async () => {
		session.me = me();
		await open('/board?story=ABC-1');
		dialog.props!.oncreated(story({ key: 'ABC-612', kind: 'bug' }), null);
		flushSync();
		nav('http://localhost/board?story=ABC-1', 'http://localhost/board');
		flushSync();
		expect(status()).toContain('Filed bug ABC-612');
	});

	it('signing out discards the draft and its stored copy', async () => {
		session.me = me();
		await open('/board');
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.title = 'Checkout fails';
		flushSync();
		expect(sessionStorage.getItem('reeve.report-draft')).not.toBeNull();
		const trigger = [...document.body.querySelectorAll('button')].find((b) => /^Account:/.test(b.getAttribute('aria-label') ?? ''))!;
		trigger.click();
		flushSync();
		([...document.body.querySelectorAll('[role="menuitem"]')].find((i) => i.textContent === 'Sign out') as HTMLElement).click();
		await vi.waitFor(() => expect(report.draft).toBeNull());
		expect(sessionStorage.getItem('reeve.report-draft')).toBeNull();
	});
});

describe('top bar', () => {
	it('publishes its height as --top-bar-height and follows a resize', async () => {
		let notify: (() => void) | null = null;
		let observed: Element | null = null;
		vi.stubGlobal(
			'ResizeObserver',
			class {
				constructor(cb: () => void) {
					notify = cb;
				}
				observe(el: Element) {
					observed = el;
				}
				disconnect() {}
			}
		);
		await open('/board');
		const bar = await vi.waitFor(() => document.body.querySelector<HTMLElement>('header.top-bar')!);
		expect(observed).toBe(bar);
		Object.defineProperty(bar, 'offsetHeight', { configurable: true, value: 57 });
		notify!();
		expect(document.documentElement.style.getPropertyValue('--top-bar-height')).toBe('57px');
		Object.defineProperty(bar, 'offsetHeight', { configurable: true, value: 101 }); // wrapped to two rows
		notify!();
		expect(document.documentElement.style.getPropertyValue('--top-bar-height')).toBe('101px');
		unmount(app!); // the effect's own cleanup removes the property
		app = null;
		expect(document.documentElement.style.getPropertyValue('--top-bar-height')).toBe('');
	});
});

describe('untriaged link', () => {
	const link = () => document.querySelector<HTMLAnchorElement>('header.top-bar a.untriaged');
	const asked = () => net.gets.filter((g) => g === 'GET /api/v1/stories').length;
	const refresh = async () => {
		for (const fn of live.fns) await fn(); // a live change or a reconnect
	};
	beforeEach(() => {
		session.me = me();
	});
	it('shows "N untriaged" linking to /triage, and refreshes on a live change', async () => {
		net.untriaged = 2;
		await open('/board');
		await vi.waitFor(() => expect(link()?.textContent?.trim()).toBe('2 untriaged'));
		expect(link()!.getAttribute('href')).toBe('/triage');
		net.untriaged = 1;
		await refresh();
		await vi.waitFor(() => expect(link()?.textContent?.trim()).toBe('1 untriaged'));
	});
	it('is absent at zero, after the count has loaded', async () => {
		await open('/board');
		await vi.waitFor(() => expect(asked()).toBeGreaterThan(0));
		await settle();
		flushSync();
		expect(link()).toBeNull();
	});
	it('keeps the last count when a reload fails, and drops a late older answer', async () => {
		net.untriaged = 4;
		await open('/board');
		await vi.waitFor(() => expect(link()?.textContent?.trim()).toBe('4 untriaged'));
		net.untriaged = null;
		await refresh();
		await settle();
		flushSync();
		expect(link()?.textContent?.trim()).toBe('4 untriaged');
		let release = () => {};
		net.untriagedHold = new Promise<void>((r) => (release = r));
		net.untriaged = 7;
		const older = refresh(); // waits
		net.untriagedHold = null;
		net.untriaged = 3;
		await refresh(); // answers first
		release();
		await older;
		await settle();
		flushSync();
		expect(link()?.textContent?.trim()).toBe('3 untriaged');
	});
});
