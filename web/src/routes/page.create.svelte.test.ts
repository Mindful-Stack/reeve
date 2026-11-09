// @vitest-environment jsdom
// The planning page's "New initiative" form, in a dialog: horizon defaults to the first bucket, a bad month is refused locally with the
// draft kept, a failed create keeps the idempotency key for the retry, success navigates to the new initiative.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Page from './+page.svelte';
import { session } from '#lib/me.svelte.ts';
import type { Initiative } from '#lib/api/types.ts';
import { config, initiative, me } from '#lib/test/fixtures.ts';
import type { FakePage } from '#lib/test/net.ts';

const route = vi.hoisted(() => ({ fake: null as null | FakePage }));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	route.fake = fakePage({ url: '/' });
	return { page: route.fake.page };
});
const goto = vi.hoisted(() => vi.fn(async (to: string | URL, opts?: unknown) => route.fake!.goto(to, opts)));
vi.mock('$app/navigation', () => ({ goto }));

type Posted = {
	idempotency_key: string;
	title: string;
	horizon: string;
	target_month: string | null;
	brief: string | null;
	owner: string | null;
};

const net = vi.hoisted(() => ({
	posts: [] as Posted[],
	urgent: [] as { method: string; path: string; body: unknown }[],
	state: { failNext: 0, rejectNext: 0, initiatives: [] as Initiative[], hold: null as Promise<void> | null }
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	const { initiative } = await import('#lib/test/fixtures.ts');
	const state = net.state;
	installFetch(async ({ method, path, body }) => {
		if (method === 'GET' && path === '/api/v1/initiatives') return state.initiatives;
		if (method === 'GET' && path === '/api/v1/decisions') return [];
		if (method === 'GET' && path === '/api/v1/war-room') return [];
		if (/^\/api\/v1\/initiatives\/[^/]+\/urgent$/.test(path)) {
			net.urgent.push({ method, path, body });
			return {};
		}
		const patch = path.match(/^\/api\/v1\/initiatives\/([^/]+)$/);
		if (method === 'PATCH' && patch) {
			const i = state.initiatives.find((x) => x.key === patch[1])!;
			i.horizon = (body as { horizon: string }).horizon;
			i.version = i.version + 1;
			return i;
		}
		if (method === 'POST' && path === '/api/v1/initiatives') {
			const posted = body as Posted;
			net.posts.push(posted);
			if (state.hold) await state.hold;
			if (state.rejectNext > 0) {
				state.rejectNext--;
				throw new TypeError('Failed to fetch');
			}
			if (state.failNext > 0) {
				state.failNext--;
				return failure('boom');
			}
			return initiative({ key: 'SMK-7', title: posted.title, horizon: posted.horizon });
		}
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.posts.length = 0;
	net.urgent.length = 0;
	net.state.failNext = 0;
	net.state.rejectNext = 0;
	net.state.initiatives = [];
	net.state.hold = null;
	route.fake!.reset();
	session.me = null;
	goto.mockClear();
});

const q = <T extends Element>(sel: string) => document.body.querySelector(sel) as T;
function field(label: string): HTMLInputElement {
	const l = [...document.body.querySelectorAll('form[aria-label="New initiative"] label')].find((x) =>
		x.textContent?.trim().startsWith(label)
	);
	return l!.querySelector('input,select,textarea') as HTMLInputElement;
}
function type(el: HTMLInputElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	el.dispatchEvent(new Event('change', { bubbles: true }));
	flushSync();
}
function submit() {
	q<HTMLFormElement>('form[aria-label="New initiative"]').requestSubmit();
}

async function open() {
	session.config = config({ horizon_buckets: ['Now', 'Next'] });
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.textContent).toContain('Now');
	});
	newButton().click();
	flushSync();
	expect(document.body.querySelector('dialog form[aria-label="New initiative"]')).not.toBeNull();
}

function newButton(): HTMLButtonElement {
	return [...document.body.querySelectorAll('button')].find((b) => b.textContent === 'New initiative')!;
}

describe('planning page: new initiative', () => {
	it('defaults the horizon to the first configured bucket', async () => {
		await open();
		expect(field('Horizon').value).toBe('Now');
	});

	it('refuses a malformed month locally and keeps the draft', async () => {
		await open();
		type(field('Title'), 'Created');
		type(field('Target month'), '2026-13');
		submit();
		flushSync();
		expect(q('form[aria-label="New initiative"] [role="alert"]').textContent).toContain('YYYY-MM');
		expect(net.posts).toHaveLength(0);
		expect(field('Title').value).toBe('Created');
	});

	it('keeps the idempotency key across a failed create, then goes to the new initiative', async () => {
		await open();
		type(field('Title'), 'Created');
		type(field('Target month'), '2026-12');
		net.state.failNext = 1;
		submit();
		await vi.waitFor(() => {
			flushSync();
			expect(q('form[aria-label="New initiative"] [role="alert"]').textContent).toContain('boom');
		});
		expect(field('Title').value).toBe('Created');
		submit();
		await vi.waitFor(() => expect(goto).toHaveBeenCalledWith('/initiatives/SMK-7'));
		expect(net.posts).toHaveLength(2);
		expect(net.posts[1].idempotency_key).toBe(net.posts[0].idempotency_key);
		expect(net.posts[1]).toEqual({
			idempotency_key: expect.any(String),
			title: 'Created',
			horizon: 'Now',
			target_month: '2026-12',
			brief: null,
			owner: null
		});
	});

	it('mints a new key for the next create after a success', async () => {
		await open();
		type(field('Title'), 'Created');
		submit();
		await vi.waitFor(() => expect(goto).toHaveBeenCalledTimes(1));
		newButton().click();
		flushSync();
		type(field('Title'), 'Created');
		submit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(2));
		expect(net.posts[1].idempotency_key).not.toBe(net.posts[0].idempotency_key);
	});

	it('resets the draft after a success', async () => {
		await open();
		type(field('Title'), 'Created');
		type(field('Horizon'), 'Next');
		submit();
		await vi.waitFor(() => expect(goto).toHaveBeenCalledTimes(1));
		newButton().click();
		flushSync();
		expect(field('Title').value).toBe('');
		expect(field('Horizon').value).toBe('Now');
	});

	it('sends one request for a double submit', async () => {
		await open();
		type(field('Title'), 'Created');
		submit();
		submit();
		await vi.waitFor(() => expect(goto).toHaveBeenCalledTimes(1));
		expect(net.posts).toHaveLength(1);
	});

	it('mints a new key when the text changes after a failed send', async () => {
		await open();
		type(field('Title'), 'Created');
		net.state.failNext = 1;
		submit();
		await vi.waitFor(() => {
			flushSync();
			expect(q('form[aria-label="New initiative"] [role="alert"]').textContent).toContain('boom');
		});
		type(field('Title'), 'Created again');
		submit();
		await vi.waitFor(() => expect(goto).toHaveBeenCalledTimes(1));
		expect(net.posts[1].idempotency_key).not.toBe(net.posts[0].idempotency_key);
		expect(net.posts[1].title).toBe('Created again');
	});

	it('keeps the draft and the key when the request is rejected, and shows the error', async () => {
		await open();
		type(field('Title'), 'Created');
		net.state.rejectNext = 1;
		submit();
		await vi.waitFor(() => {
			flushSync();
			expect(q('form[aria-label="New initiative"] [role="alert"]').textContent).toContain('Failed to fetch');
		});
		expect(field('Title').value).toBe('Created');
		submit();
		await vi.waitFor(() => expect(goto).toHaveBeenCalledTimes(1));
		expect(net.posts[1].idempotency_key).toBe(net.posts[0].idempotency_key);
	});

	for (const how of ['Close', 'Escape'] as const) {
		it(`keeps the dialog open on ${how} while the create is pending`, async () => {
			await open();
			type(field('Title'), 'Created');
			let release!: () => void;
			net.state.hold = new Promise<void>((r) => (release = r));
			submit();
			await vi.waitFor(() => expect(net.posts).toHaveLength(1));
			flushSync();
			const dialog = q<HTMLDialogElement>('dialog');
			if (how === 'Close') {
				const close = q<HTMLButtonElement>('dialog button[aria-label="Close"]');
				expect(close.disabled).toBe(true);
				close.click();
			} else expect(dialog.dispatchEvent(new Event('cancel', { cancelable: true }))).toBe(false);
			flushSync();
			expect(q('dialog form[aria-label="New initiative"]')).not.toBeNull();
			release();
			await vi.waitFor(() => expect(goto).toHaveBeenCalledWith('/initiatives/SMK-7'));
		});
	}

	it('clears a stale error on Cancel and returns focus to the toggle', async () => {
		await open();
		type(field('Title'), 'Created');
		net.state.failNext = 1;
		submit();
		await vi.waitFor(() => {
			flushSync();
			expect(q('form[aria-label="New initiative"] [role="alert"]').textContent).toContain('boom');
		});
		const toggle = newButton();
		[...document.body.querySelectorAll('button')].find((b) => b.textContent === 'Cancel')!.click();
		await vi.waitFor(() => expect(document.activeElement).toBe(toggle));
		toggle.click();
		flushSync();
		expect(document.body.querySelector('form[aria-label="New initiative"] [role="alert"]')).toBeNull();
	});
});

describe('planning page: All or Mine', () => {
	const ini = (key: string, title: string, owner: string) => initiative({ key, title, horizon: 'Now', owner });

	async function show(search: string) {
		await route.fake!.goto(`/${search}`);
		session.config = config({ horizon_buckets: ['Now', 'Next'] });
		session.me = me({ email: 'me@example.com' });
		net.state.initiatives = [ini('R-1', 'Mine one', 'me@example.com'), ini('R-2', 'Theirs one', 'you@example.com')];
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Mine one');
		});
	}
	const pressed = (label: string) =>
		[...document.body.querySelectorAll<HTMLButtonElement>('[aria-label="Show"] button')].find((b) => b.textContent === label)!;

	it('reads ?mine=1 and shows only the signed-in user\'s initiatives', async () => {
		await show('?mine=1');
		expect(pressed('Mine').getAttribute('aria-pressed')).toBe('true');
		expect(document.body.textContent).not.toContain('Theirs one');
	});

	it('shows everything without the parameter, and Mine writes ?mine=1 into the URL in place', async () => {
		await show('');
		expect(document.body.textContent).toContain('Theirs one');
		pressed('Mine').click();
		expect(goto).toHaveBeenCalledTimes(1);
		expect(String(goto.mock.calls[0][0])).toBe('http://localhost/?mine=1');
		expect(goto.mock.calls[0][1]).toEqual({ replace: true, reset: false });
	});

	it('All removes the parameter', async () => {
		await show('?mine=1');
		pressed('All').click();
		expect(String(goto.mock.calls[0][0])).toBe('http://localhost/');
	});
});

describe('planning page: moves', () => {
	it('focuses the moved card\'s menu trigger in its new column after a menu move', async () => {
		session.config = config({ horizon_buckets: ['Now', 'Next', 'Later'] });
		net.state.initiatives = [initiative({ key: 'R-1', title: 'Moving', horizon: 'Now' })];
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Moving');
		});
		q<HTMLButtonElement>('button[aria-label="Actions for R-1"]').click();
		flushSync();
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === 'Move to Next')!.click();
		await vi.waitFor(() => {
			flushSync();
			const trigger = q('section[aria-label="Next"] [data-testid="card-R-1"] [data-control="menu"]');
			expect(trigger).not.toBeNull();
			expect(document.activeElement).toBe(trigger);
		});
	});

	it('expands the last horizon in configured order, before Other, and points the toggle at it', async () => {
		session.config = config({ horizon_buckets: ['Now', 'Next', 'Later'] });
		net.state.initiatives = [initiative({ key: 'R-1', title: 'Stray', horizon: 'Someday' })];
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Stray');
		});
		const names = () => [...document.body.querySelectorAll('section.column')].map((c) => c.getAttribute('aria-label'));
		expect(names()).toEqual(['Now', 'Next', 'Other']);
		const toggle = q<HTMLButtonElement>('button.collapsed');
		toggle.click();
		flushSync();
		expect(names()).toEqual(['Now', 'Next', 'Later', 'Other']);
		expect(document.getElementById(toggle.getAttribute('aria-controls')!)!.getAttribute('aria-label')).toBe('Later');
	});
});


describe('planning page: urgent marks', () => {
	const mark = { at: '2026-11-09T09:00:00Z', by: 'dana@example.com', reason: null, expires_at: '2026-11-23T09:00:00Z', active: true, done: false };
	async function loaded() {
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Hot one');
		});
	}
	const item = (label: string) =>
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === label)!;

	it('keeps an urgent initiative in its column with a chip reading Urgent', async () => {
		session.config = config({ horizon_buckets: ['Now', 'Next'] });
		net.state.initiatives = [
			initiative({ key: 'U-1', title: 'Hot one', horizon: 'Now', urgent_mark: mark, urgent_sources: [{ key: 'U-1', item: 'initiative' }] })
		];
		app = mount(Page, { target: document.body });
		await loaded();
		const card = q('section[aria-label="Now"] [data-testid="card-U-1"]');
		expect(card.querySelector('.urgent-chip')!.textContent).toBe('Urgent');
		expect(q('section[aria-label="Now"] h2').textContent).toContain('1');
	});

	it('marks from the card menu with a reason and clears through the same menu', async () => {
		session.config = config({ horizon_buckets: ['Now', 'Next'] });
		net.state.initiatives = [initiative({ key: 'U-1', title: 'Hot one', horizon: 'Now', urgent_mark: mark })];
		app = mount(Page, { target: document.body });
		await loaded();
		q<HTMLButtonElement>('button[aria-label="Actions for U-1"]').click();
		flushSync();
		item('Mark urgent…').click();
		flushSync();
		type(q<HTMLInputElement>('dialog input'), 'Outage');
		q<HTMLFormElement>('form[aria-label="Mark urgent"]').requestSubmit();
		await vi.waitFor(() => expect(net.urgent).toHaveLength(1));
		expect(net.urgent[0]).toEqual({ method: 'POST', path: '/api/v1/initiatives/U-1/urgent', body: { reason: 'Outage' } });
		await vi.waitFor(() => expect(document.body.querySelector('dialog')).toBeNull());
		q<HTMLButtonElement>('button[aria-label="Actions for U-1"]').click();
		flushSync();
		item('Clear urgent mark').click();
		await vi.waitFor(() => expect(net.urgent).toHaveLength(2));
		expect(net.urgent[1]).toMatchObject({ method: 'DELETE', path: '/api/v1/initiatives/U-1/urgent' });
	});
});
