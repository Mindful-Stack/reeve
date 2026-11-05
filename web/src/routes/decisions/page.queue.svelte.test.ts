// @vitest-environment jsdom
// The decisions page is the queue (GET /decisions/queue): All by default with Mine as the toggle (?mine=1), the
// server's summary line, its four groups in server order (Closed folded unless ?closed=1) and each decision's blocks.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BlockedStory, DecisionQueue, Epic, QueueGroup, RequiredDecision } from '#lib/api/types.ts';
import { decision } from '#lib/test/fixtures.ts';
import type { FakePage } from '#lib/test/net.ts';
import Page from './+page.svelte';

const route = vi.hoisted(() => ({ fake: null as null | FakePage }));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	route.fake = fakePage({ url: '/decisions' });
	return { page: route.fake.page };
});
const goto = vi.hoisted(() => vi.fn(async (to: string | URL, opts?: unknown) => route.fake!.goto(to, opts)));
vi.mock('$app/navigation', () => ({ goto }));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: () => {},
	everyVisible: () => {}
}));
vi.mock('#lib/me.svelte.ts', async (orig) => {
	const real = await orig<typeof import('#lib/me.svelte.ts')>();
	const { me } = await import('#lib/test/fixtures.ts');
	real.session.me = me({ email: 'dan@example.com' });
	return real;
});

const empty: DecisionQueue = { entries: [], summary: { open: 0, blocking: 0, blocked_stories: 0, overdue: 0 } };
const net = vi.hoisted(() => ({
	queue: null as null | DecisionQueue,
	queries: [] as string[],
	paths: [] as string[],
	fail: null as null | string,
	closes: [] as unknown[],
	/** What the queue answers once a close lands. */
	afterClose: null as null | DecisionQueue
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	installFetch(({ method, url, path, body }) => {
		net.paths.push(path);
		if (net.fail) return failure(net.fail);
		if (path === '/api/v1/decisions/queue') {
			net.queries.push(url.search);
			// Like the server: closed decisions only when asked for.
			if (url.searchParams.get('include_closed') === 'true') return net.queue;
			return { ...net.queue!, entries: net.queue!.entries.filter((e) => e.group !== 'closed') };
		}
		const close = path.match(/^\/api\/v1\/decisions\/(\d+)\/close$/);
		if (method === 'POST' && close) {
			net.closes.push(body);
			net.queue = net.afterClose ?? net.queue;
			return net.queue!.entries.find((e) => e.decision.id === Number(close[1]))!.decision;
		}
		if (path === '/api/v1/initiatives') return [];
		if (path === '/api/v1/epics') {
			const epics: Epic[] = [{ key: 'RV-2', title: 'E', version: 1, brief: '' }];
			return epics;
		}
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

const NOW = Date.parse('2026-10-27T12:00:00Z');
function dec(id: number, title: string, more: Partial<RequiredDecision> = {}): RequiredDecision {
	return decision({ id, on: 'RV-2', on_type: 'epic', title, decider: 'dan@example.com', ...more });
}
const record: RequiredDecision['record'] = { repo: 'kb', path: 'adr/1.md', record_id: 'kb/ADR-1' };
const accepted = { kind: 'accepted' as const, commit: 'abcdef0123' };
const closedBy = { at: '2026-10-02T00:00:00Z', by: 'dan@example.com', reason: 'moot', kind: 'withdrawn' as const };
function entry(group: QueueGroup, d: RequiredDecision, blocks: BlockedStory[] = []) {
	return { group, decision: d, blocks };
}
function queue(entries: ReturnType<typeof entry>[], summary = empty.summary): DecisionQueue {
	return { entries, summary };
}

let app: ReturnType<typeof mount> | null = null;
beforeEach(() => {
	vi.useFakeTimers({ toFake: ['Date'] });
	vi.setSystemTime(NOW);
	net.queue = empty;
});
afterEach(() => {
	vi.useRealTimers();
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	route.fake!.reset();
	goto.mockClear();
	net.queries.length = 0;
	net.paths.length = 0;
	net.fail = null;
	net.closes.length = 0;
	net.afterClose = null;
});

async function open() {
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('h1')).not.toBeNull();
		expect(document.body.textContent).not.toContain('Loading…');
	});
}
const text = (el: Element | null | undefined) => el?.textContent?.replace(/\s+/g, ' ').trim();
const titles = (sel: string) => [...document.body.querySelectorAll(`${sel} .title`)].map((t) => t.textContent);
/** Each group heading's words, without its count or the fold's arrow. */
const headings = () => [...document.body.querySelectorAll('h2')].map((h) => text(h)?.replace(/^[▸▾] /, '').replace(/ \d+$/, ''));
const pressed = () => document.body.querySelector('[role="group"][aria-label="Decider"] [aria-pressed="true"]')?.textContent;

describe('decisions queue page', () => {
	it('requests the queue unscoped and shows every decider by default', async () => {
		net.queue = queue([entry('blocking', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
		await open();
		expect(document.body.querySelector('h1')?.textContent).toBe('Decisions');
		expect(pressed()).toBe('All');
		expect(net.paths).not.toContain('/api/v1/decisions');
		expect(net.queries).toEqual(['?include_closed=true']);
		expect(titles('.decisions')).toEqual(['Mine', 'Theirs']);
	});

	it('ignores an old ?all=1', async () => {
		await route.fake!.goto('/decisions?all=1');
		net.queue = queue([entry('blocking', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
		await open();
		expect(pressed()).toBe('All');
		expect(titles('.decisions')).toEqual(['Mine', 'Theirs']);
	});

	it('Mine filters to the caller and writes ?mine=1', async () => {
		net.queue = queue([entry('blocking', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
		await open();
		const mine = [...document.body.querySelectorAll('[aria-label="Decider"] button')].find((b) => b.textContent === 'Mine') as HTMLButtonElement;
		mine.click();
		expect(String(goto.mock.calls.at(-1)?.[0])).toContain('mine=1');
		await vi.waitFor(() => {
			flushSync();
			expect(pressed()).toBe('Mine');
			expect(titles('.decisions')).toEqual(['Mine']);
		});
	});

	it('opens on Mine from ?mine=1', async () => {
		await route.fake!.goto('/decisions?mine=1');
		net.queue = queue([entry('blocking', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
		await open();
		expect(pressed()).toBe('Mine');
		expect(titles('.decisions')).toEqual(['Mine']);
		expect(document.body.textContent).not.toContain('Theirs');
	});

	it("says the server's summary", async () => {
		net.queue = queue([entry('blocking', dec(1, 'One'))], { open: 6, blocking: 4, blocked_stories: 11, overdue: 2 });
		await open();
		expect(text(document.body.querySelector('.summary'))).toBe('6 open · 4 blocking 11 stories · 2 overdue');
	});

	it("groups by the server's groups, in server order, with Closed folded", async () => {
		// Groups that disagree with what blocking() would compute: an accepted record listed as blocking, an unchecked
		// decision listed as satisfied. The page must follow the server.
		net.queue = queue([
			entry('blocking', dec(2, 'Accepted but blocking', { record, check: accepted })),
			entry('blocking', dec(1, 'Second blocking')),
			entry('open', dec(4, 'Not blocking yet')),
			entry('satisfied', dec(5, 'Unchecked but satisfied')),
			entry('closed', dec(6, 'Old one', { closed: closedBy }))
		]);
		await open();
		expect(headings()).toEqual(['Blocking work', 'Open, not blocking yet', 'Accepted', 'Closed']);
		expect(titles('#decisions-blocking')).toEqual(['Accepted but blocking', 'Second blocking']);
		expect(titles('#decisions-open')).toEqual(['Not blocking yet']);
		expect(titles('#decisions-satisfied')).toEqual(['Unchecked but satisfied']);
		expect(document.getElementById('decisions-closed')).toBeNull();
		expect(document.body.textContent).not.toContain('Old one');
		const fold = document.body.querySelector<HTMLButtonElement>('h2 button[aria-expanded]')!;
		expect(text(fold)).toContain('Closed');
		expect(fold.getAttribute('aria-expanded')).toBe('false');
	});

	it('keeps a decision withdrawn in the default view on screen with its outcome, its group folded', async () => {
		net.queue = queue([entry('blocking', dec(1, 'Gate')), entry('open', dec(4, 'Free'))]);
		net.afterClose = queue([
			entry('open', dec(4, 'Free')),
			entry('closed', dec(1, 'Gate', { closed: { ...closedBy, reason: 'no longer needed' }, version: 2 }))
		]);
		await open();
		const row = () => [...document.body.querySelectorAll('.decisions > li')].find((r) => r.querySelector('.title')?.textContent === 'Gate')!;
		row().querySelector<HTMLButtonElement>('button[aria-label^="Actions for Gate"]')!.click();
		flushSync();
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.trim() === 'Withdraw')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement?.getAttribute('data-field')).toBe('reason');
		});
		const reason = document.activeElement as HTMLInputElement;
		reason.value = 'no longer needed';
		reason.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		reason.form!.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(net.closes).toHaveLength(1);
			expect(row()?.textContent).toContain('Withdrawn by');
		});
		// Pinned where it was shown while it holds focus: the Closed group itself stays folded.
		expect(row().closest('ul')!.id).toBe('decisions-blocking');
		// Counted where it is shown, though the server now puts it in Closed.
		expect(text(document.body.querySelector('h2'))).toBe('Blocking work 1');
		expect(row().contains(document.activeElement)).toBe(true);
		expect(document.getElementById('decisions-closed')).toBeNull();
		// Once focus leaves, it folds into the collapsed Closed group.
		(document.activeElement as HTMLElement).blur();
		await vi.waitFor(() => {
			flushSync();
			expect(row()).toBeUndefined();
		});
		expect(document.body.querySelector('h2 button[aria-expanded]')?.getAttribute('aria-expanded')).toBe('false');
		expect(document.getElementById('decisions-closed')).toBeNull();
	});

	it('says the queue is empty when only folded closed decisions are listed', async () => {
		net.queue = queue([entry('closed', dec(6, 'Old one', { closed: closedBy }))]);
		await open();
		expect(document.body.textContent).toContain('No open decisions');
		expect(headings()).toEqual(['Closed']);
	});

	it('renders no heading for an empty group', async () => {
		net.queue = queue([entry('blocking', dec(1, 'Gate')), entry('satisfied', dec(2, 'Settled', { record, check: accepted }))]);
		await open();
		expect(headings()).toEqual(['Blocking work', 'Accepted']);
		expect(document.getElementById('decisions-open')).toBeNull();
	});

	it('Show closed unfolds the closed ones', async () => {
		await route.fake!.goto('/decisions?closed=1');
		net.queue = queue([entry('satisfied', dec(2, 'Settled', { record, check: accepted })), entry('closed', dec(6, 'Old one', { closed: closedBy }))]);
		await open();
		expect(net.queries).toEqual(['?include_closed=true']);
		const box = [...document.body.querySelectorAll('label')].find((l) => l.textContent?.trim() === 'Show closed')!.querySelector('input')!;
		expect(box.checked).toBe(true);
		expect(titles('#decisions-closed')).toEqual(['Old one']);
		expect(document.body.querySelector('h2 button[aria-expanded]')?.getAttribute('aria-expanded')).toBe('true');
	});

	it("passes each decision's blocks to its card", async () => {
		const blocks = [
			{ key: 'RV-7', title: 'Seven' },
			{ key: 'RV-8', title: 'Eight' }
		];
		net.queue = queue([entry('blocking', dec(1, 'Gate'), blocks), entry('open', dec(4, 'Free'))]);
		await open();
		const lines = [...document.body.querySelectorAll('button[data-field="blocks"]')].map((l) => text(l));
		expect(lines).toEqual(['Blocks 2 stories ▸']);
		expect(document.getElementById('decisions-blocking')!.textContent).toContain('Blocks 2 stories');
	});

	it('says how fresh the newest record check is', async () => {
		net.queue = queue([
			entry('satisfied', dec(2, 'Settled', { record, check: accepted, checked_at: '2026-10-27T11:57:00Z' })),
			entry('blocking', dec(4, 'Proposed', { record, check: { kind: 'proposed_pr_open' }, checked_at: '2026-10-27T11:00:00Z' }))
		]);
		await open();
		expect(document.body.querySelector('.checked-all')?.textContent?.trim()).toBe('Records checked 3m ago');
	});

	it('says so when the queue is empty', async () => {
		await open();
		expect(document.body.textContent).toContain('No open decisions');
		expect(document.body.querySelector('.checked-all')).toBeNull();
	});

	describe('the UTC date changing on an idle page', () => {
		// The server's overdue ordering and summary are by UTC date: a new day refetches, with no activity and no record.
		let state: DocumentVisibilityState = 'visible';
		beforeEach(() => {
			vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
			vi.setSystemTime(Date.parse('2026-10-27T23:58:30Z'));
			state = 'visible';
			Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
			net.queue = queue([entry('open', dec(4, 'Free', { due: '2026-10-27' }))]);
		});
		afterEach(() => {
			delete (document as unknown as Record<string, unknown>).visibilityState;
		});

		it('refetches once the clock crosses UTC midnight, and not before', async () => {
			await open();
			expect(net.queries).toHaveLength(1);
			vi.advanceTimersByTime(60_000); // 23:59:30
			flushSync();
			expect(net.queries).toHaveLength(1);
			vi.advanceTimersByTime(60_000); // 00:00:30 the next day
			await vi.waitFor(() => {
				flushSync();
				expect(net.queries).toHaveLength(2);
			});
			vi.advanceTimersByTime(5 * 60_000);
			flushSync();
			expect(net.queries).toHaveLength(2);
		});

		it('a hidden tab refetches when it is shown again on a new day', async () => {
			await open();
			state = 'hidden';
			document.dispatchEvent(new Event('visibilitychange'));
			vi.advanceTimersByTime(10 * 60_000);
			flushSync();
			expect(net.queries).toHaveLength(1);
			state = 'visible';
			document.dispatchEvent(new Event('visibilitychange'));
			await vi.waitFor(() => {
				flushSync();
				expect(net.queries).toHaveLength(2);
			});
		});

		it('a tab shown again on the same day does not refetch', async () => {
			await open();
			state = 'hidden';
			document.dispatchEvent(new Event('visibilitychange'));
			state = 'visible';
			document.dispatchEvent(new Event('visibilitychange'));
			flushSync();
			await Promise.resolve();
			expect(net.queries).toHaveLength(1);
		});
	});

	it('shows a load failure as an error notice with Retry', async () => {
		net.fail = 'boom';
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('.notice.error')?.textContent).toContain('Could not load the decisions: boom');
		});
		expect([...document.body.querySelectorAll('.notice button')].map((b) => b.textContent)).toContain('Retry');
	});
});
