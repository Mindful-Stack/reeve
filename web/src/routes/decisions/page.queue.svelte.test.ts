// @vitest-environment jsdom
// The decisions page is the queue (GET /decisions/queue): All by default with Mine as the toggle (?mine=1), the
// server's summary line, its seven lanes in server order (Closed folded unless ?closed=1) and each decision's blocks.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AffectedStory, DecisionQueue, Epic, QueueGroup, QueueLane, RequiredDecision } from '#lib/api/types.ts';
import { resetDecisionDraftsForTests } from '#lib/decisionDrafts.svelte.ts';
import { blockedStory, decision, discoveryEntry, queueEntry, queueSummary } from '#lib/test/fixtures.ts';
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

const empty: DecisionQueue = { entries: [], discoveries: [], summary: queueSummary() };
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
			const epics: Epic[] = [{ key: 'RV-2', title: 'E', version: 1, brief: '', urgent_mark: null, urgent_sources: [], promise: null }];
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
const GROUP: Record<QueueLane, QueueGroup> = {
	review: 'blocking',
	this_sprint: 'blocking',
	next_sprint: 'blocking',
	beyond: 'blocking',
	open: 'open',
	satisfied: 'satisfied',
	closed: 'closed'
};
function entry(lane: QueueLane, d: RequiredDecision, blocks: AffectedStory[] = [], more: Partial<ReturnType<typeof queueEntry>> = {}) {
	return queueEntry({ group: GROUP[lane], lane, decision: d, blocks, ...more });
}
function queue(entries: ReturnType<typeof entry>[], summary = empty.summary): DecisionQueue {
	return { entries, discoveries: [], summary };
}

let app: ReturnType<typeof mount> | null = null;
beforeEach(() => {
	vi.useFakeTimers({ toFake: ['Date'] });
	vi.setSystemTime(NOW);
	net.queue = empty;
});
afterEach(() => {
	vi.useRealTimers();
	resetDecisionDraftsForTests();
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
	const NEXT = { kind: 'write_record' } as const;
	it('groups by page lane under the lane headings, with anchors on the two sprint lanes', async () => {
		net.queue = queue([
			entry('this_sprint', dec(1, 'Reviewing'), [], { next_step: { kind: 'review_record', pr_url: 'https://github.com/o/r/pull/3' }, needed_in: 'this_sprint' }),
			entry('this_sprint', dec(2, 'Needed'), [], { next_step: NEXT, needed_in: 'this_sprint' }),
			entry('next_sprint', dec(3, 'Soon')),
			entry('beyond', dec(4, 'Ranked'), [], { needed_in: 'beyond', needed_in_reason: { kind: 'held' } }),
			entry('beyond', dec(6, 'Later one'), [], { needed_in: 'beyond' }),
			entry('open', dec(5, 'Idle'))
		]);
		await open();
		expect(headings()).toEqual(['Needed this sprint', 'Needed next sprint', 'Now, no goal yet', 'Later', 'Open, not blocking yet']);
		expect([...document.body.querySelectorAll('h2')].map((h) => h.id)).toEqual(['this-sprint', 'next-sprint', '', '', '']);
		// A record awaiting review stays in its window's lane: there is no Review lane.
		expect(titles('#decisions-this_sprint')).toEqual(['Reviewing', 'Needed']);
		expect(titles('#decisions-now')).toEqual(['Ranked']);
		expect(titles('#decisions-later')).toEqual(['Later one']);
		expect(text(document.getElementById('decisions-this_sprint'))).toContain('Write a record');
		expect(text(document.getElementById('decisions-this_sprint'))).toContain('Linked PR');
		expect(document.body.querySelector('#decisions-this_sprint a.title')?.getAttribute('href')).toBe('/decisions?decision=1');
	});

	it('puts the divider before the first lane no goal or promise needs, once', async () => {
		net.queue = queue([entry('this_sprint', dec(1, 'Now')), entry('open', dec(2, 'Idle')), entry('satisfied', dec(3, 'Settled'))]);
		await open();
		const dividers = [...document.body.querySelectorAll('.divider')];
		expect(dividers.map((d) => text(d))).toEqual(['No goal or promise needs these yet']);
		expect(dividers[0].nextElementSibling?.tagName).toBe('H2');
		expect(text(dividers[0].nextElementSibling)).toContain('Open, not blocking yet');
		expect(document.body.textContent).not.toContain('Nothing needed this sprint or next.');
	});

	it('skips an empty lane for the divider (Now has rows: it leads)', async () => {
		net.queue = queue([entry('beyond', dec(1, 'Ranked'), [], { needed_in_reason: { kind: 'held' } }), entry('open', dec(2, 'Idle'))]);
		await open();
		const divider = document.body.querySelector('.divider')!;
		expect(text(divider.nextElementSibling)).toContain('Now, no goal yet');
	});

	it('does not say nothing is needed on a completely empty queue', async () => {
		await open();
		expect(document.body.textContent).toContain('Nothing holds work back');
		expect(document.body.querySelector('.quiet')).toBeNull();
	});

	it('words the empty line for the caller under ?mine=1', async () => {
		await route.fake!.goto('/decisions?mine=1');
		net.queue = queue([entry('beyond', dec(1, 'Later'))]);
		await open();
		expect(text(document.body.querySelector('.quiet'))).toBe('Nothing of yours needed this sprint or next.');
	});

	it('shows a discovery row: its item link, status, holds, reason and its nested decisions in a list of their own', async () => {
		const goal = { kind: 'goal' as const, goal_id: 3, outcome: 'Ship tiers' };
		net.queue = {
			entries: [
				entry('this_sprint', dec(1, 'Pick tiers'), [], { under: 'RV-2', needed_in: 'this_sprint', needed_in_reason: goal }),
				entry('beyond', dec(2, 'Standalone'), [blockedStory({ key: 'RV-9' })], { needed_in: 'beyond', needed_in_reason: { kind: 'held' } })
			],
			discoveries: [
				discoveryEntry({
					key: 'RV-2',
					level: 'epic',
					title: 'Tiers',
					also: 'RV-1',
					lane: 'this_sprint',
					needed_in_reason: goal,
					blocks: [blockedStory({ key: 'RV-7', title: 'Seven' })],
					under_way: [blockedStory({ key: 'RV-8', title: 'Eight', delivery: 'in_progress' })],
					at: 0
				}),
				discoveryEntry({
					key: 'RV-5',
					level: 'initiative',
					title: 'Billing',
					lane: 'beyond',
					needed_in_reason: { kind: 'held' },
					under_way: [blockedStory({ key: 'RV-6', delivery: 'in_progress' })],
					at: 1
				})
			],
			summary: queueSummary({ this_sprint: 1 })
		};
		await open();
		expect(headings()).toEqual(['Needed this sprint', 'Now, no goal yet']);
		// The row's nested decision counts once under its lane; Now counts the row and the standalone decision.
		expect(text(document.getElementById('this-sprint'))).toBe('Needed this sprint 1');
		expect(text([...document.body.querySelectorAll('h2')][1])).toBe('Now, no goal yet 2');
		const row = document.body.querySelector('[data-discovery="RV-2"]')!;
		expect(row.querySelector('a.item')?.getAttribute('href')).toBe('/epics/RV-2');
		expect(row.querySelector('a.item .key')?.textContent).toBe('RV-2');
		expect(text(row.querySelector('a.item'))).toContain('Tiers');
		expect(text(row.querySelector('.status'))).toBe('in discovery · with RV-1');
		expect(row.querySelector('.status a')?.getAttribute('href')).toBe('/initiatives/RV-1');
		expect(text(row.querySelector('[data-testid="reason"]'))).toBe('for goal: Ship tiers');
		const holds = row.querySelector<HTMLButtonElement>('.holds button')!;
		expect(text(holds)).toBe('Holds 1 story · 1 under way ▸');
		holds.click();
		flushSync();
		expect([...row.querySelectorAll('.stories a')].map((a) => a.getAttribute('href'))).toEqual(['/board?story=RV-7', '/board?story=RV-8']);
		// The nested decision is listed in the row's own list, right after it, with no line of its own.
		const nested = document.getElementById('decisions-row-RV-2')!;
		expect(row.nextElementSibling).toBe(nested);
		expect(titles('#decisions-row-RV-2')).toEqual(['Pick tiers']);
		expect(nested.querySelector('.needed')).toBeNull();
		// A Now row holding none reads off its own lists; the standalone decision after it says its own Now line.
		const now = document.body.querySelector('[data-discovery="RV-5"]')!;
		expect(now.querySelector('a.item')?.getAttribute('href')).toBe('/initiatives/RV-5');
		expect(text(now.querySelector('.holds button'))).toBe('1 under way ▸');
		expect(text(now.querySelector('[data-testid="reason"]'))).toBe('Now · holds none, 1 under way');
		expect(text(document.getElementById('decisions-now-after-RV-5')?.querySelector('.needed'))).toBe('Now · holds 1');
	});

	it("under Mine keeps a row that nests one of mine, lists only mine, and still dates its reason from another decider's decision", async () => {
		await route.fake!.goto('/decisions?mine=1');
		net.queue = {
			entries: [
				entry('beyond', dec(4, 'Theirs', { decider: 'bob@example.com', due: '2026-10-30' }), [], { under: 'RV-2', needed_in: 'beyond' }),
				entry('beyond', dec(8, 'Mine one'), [], { under: 'RV-2', needed_in: 'beyond' })
			],
			discoveries: [discoveryEntry({ key: 'RV-2', lane: 'beyond', needed_in_reason: { kind: 'decision_due', id: 4 }, at: 0 })],
			summary: queueSummary()
		};
		await open();
		const row = document.body.querySelector('[data-discovery="RV-2"]')!;
		expect(text(row.querySelector('[data-testid="reason"]'))).toBe('Now · decision #4 due 30 Oct');
		expect(row.querySelector('[data-testid="reason"] a')?.getAttribute('href')).toBe('/decisions?mine=1&decision=4');
		expect(titles('#decisions-row-RV-2')).toEqual(['Mine one']);
		expect(document.body.textContent).not.toContain('Theirs');
		expect(text(document.body.querySelector('.quiet'))).toBe('Nothing of yours needed this sprint or next.');
		expect(text(document.body.querySelector('.divider')?.nextElementSibling)).toContain('Now, no goal yet');
	});

	it('under Mine does not say nothing is needed when a row in this sprint nests one of mine, and says no sprint count (the lane headings carry them)', async () => {
		await route.fake!.goto('/decisions?mine=1');
		net.queue = {
			entries: [
				entry('this_sprint', dec(4, 'Theirs', { decider: 'bob@example.com' }), [], { needed_in: 'this_sprint' }),
				entry('this_sprint', dec(8, 'Mine one'), [], { under: 'RV-2', needed_in: 'this_sprint' })
			],
			discoveries: [discoveryEntry({ key: 'RV-2', lane: 'this_sprint', at: 1 })],
			summary: queueSummary({ this_sprint: 2 })
		};
		await open();
		expect(document.body.querySelector('.quiet')).toBeNull();
		expect(document.body.querySelector('.summary')).toBeNull();
		expect(document.body.querySelector('.divider')).toBeNull();
	});

	it('says nothing is needed this sprint or next when those lanes are empty', async () => {
		net.queue = queue([entry('beyond', dec(1, 'Later'))]);
		await open();
		expect(text(document.body.querySelector('.quiet'))).toBe('Nothing needed this sprint or next.');
	});

	it('counts the records to review and the overdue, leaving out a zero part', async () => {
		net.queue = queue(
			[entry('this_sprint', dec(1, 'R'), [], { next_step: { kind: 'review_record', pr_url: 'https://github.com/o/r/pull/1' } }), entry('beyond', dec(2, 'B1')), entry('beyond', dec(3, 'B2')), entry('open', dec(4, 'O'))],
			queueSummary({ open: 4, review: 1, this_sprint: 0, next_sprint: 3, overdue: 2 })
		);
		await open();
		// The lanes carry their own counts on their headings: the summary line has none of them.
		expect(text(document.body.querySelector('.summary'))).toBe('1 to review · 2 overdue');
	});

	it('words each part of the summary', async () => {
		net.queue = queue(
			[entry('beyond', dec(1, 'B'))],
			queueSummary({ review: 1, this_sprint: 2, next_sprint: 3, overdue: 5 })
		);
		await open();
		expect(text(document.body.querySelector('.summary'))).toBe('1 to review · 5 overdue');
	});

	it('has no summary line when every part is zero', async () => {
		net.queue = queue([entry('open', dec(1, 'Idle'))], queueSummary({ open: 1 }));
		await open();
		expect(document.body.querySelector('.summary')).toBeNull();
	});

	it('says a lane named by the hash is empty, only when it is', async () => {
		await route.fake!.goto('/decisions#this-sprint');
		net.queue = queue([entry('next_sprint', dec(1, 'Soon'))]);
		await open();
		expect(text(document.body.querySelector('[data-testid="empty-lane"]'))).toBe('Nothing needed in this lane right now.');
		// A lane with rows, and a hash that is not a lane, say nothing.
		await route.fake!.goto('/decisions#next-sprint');
		flushSync();
		expect(document.body.querySelector('[data-testid="empty-lane"]')).toBeNull();
		await route.fake!.goto('/decisions#somewhere');
		flushSync();
		expect(document.body.querySelector('[data-testid="empty-lane"]')).toBeNull();
	});

	it('scrolls to the lane named by the hash once the queue has rendered', async () => {
		await route.fake!.goto('/decisions#next-sprint');
		const scroll = vi.fn();
		(Element.prototype as unknown as { scrollIntoView: () => void }).scrollIntoView = scroll;
		try {
			net.queue = queue([entry('next_sprint', dec(1, 'Soon'))]);
			await open();
			await vi.waitFor(() => expect(scroll).toHaveBeenCalledTimes(1));
			expect(scroll.mock.contexts[0]).toBe(document.getElementById('next-sprint'));
		} finally {
			delete (Element.prototype as unknown as Record<string, unknown>).scrollIntoView;
		}
	});

	it('requests the queue unscoped and shows every decider by default', async () => {
		net.queue = queue([entry('this_sprint', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
		await open();
		expect(document.body.querySelector('h1')?.textContent).toBe('Readiness');
		expect(pressed()).toBe('All');
		expect(net.paths).not.toContain('/api/v1/decisions');
		expect(net.queries).toEqual(['?include_closed=true']);
		expect(titles('.decisions')).toEqual(['Mine', 'Theirs']);
	});

	it('ignores an old ?all=1', async () => {
		await route.fake!.goto('/decisions?all=1');
		net.queue = queue([entry('this_sprint', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
		await open();
		expect(pressed()).toBe('All');
		expect(titles('.decisions')).toEqual(['Mine', 'Theirs']);
	});

	it('Mine filters to the caller and writes ?mine=1', async () => {
		net.queue = queue([entry('this_sprint', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
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
		net.queue = queue([entry('this_sprint', dec(1, 'Mine')), entry('open', dec(3, 'Theirs', { decider: 'bob@example.com' }))]);
		await open();
		expect(pressed()).toBe('Mine');
		expect(titles('.decisions')).toEqual(['Mine']);
		expect(document.body.textContent).not.toContain('Theirs');
	});

	it("says the server's overdue count", async () => {
		net.queue = queue([entry('this_sprint', dec(1, 'One'))], queueSummary({ open: 6, blocking: 4, blocked_stories: 11, overdue: 2 }));
		await open();
		expect(text(document.body.querySelector('.summary'))).toBe('2 overdue');
	});

	it("groups by the server's groups, in server order, with Closed folded", async () => {
		// Groups that disagree with what blocking() would compute: an accepted record listed as blocking, an unchecked
		// decision listed as satisfied. The page must follow the server.
		net.queue = queue([
			entry('this_sprint', dec(2, 'Accepted but blocking', { record, check: accepted })),
			entry('this_sprint', dec(1, 'Second blocking')),
			entry('open', dec(4, 'Not blocking yet')),
			entry('satisfied', dec(5, 'Unchecked but satisfied')),
			entry('closed', dec(6, 'Old one', { closed: closedBy }))
		]);
		await open();
		expect(headings()).toEqual(['Needed this sprint', 'Open, not blocking yet', 'Accepted', 'Closed']);
		expect(titles('#decisions-this_sprint')).toEqual(['Accepted but blocking', 'Second blocking']);
		expect(titles('#decisions-open')).toEqual(['Not blocking yet']);
		expect(titles('#decisions-satisfied')).toEqual(['Unchecked but satisfied']);
		expect(document.getElementById('decisions-closed')).toBeNull();
		expect(document.body.textContent).not.toContain('Old one');
		const fold = document.body.querySelector<HTMLButtonElement>('h2 button[aria-expanded]')!;
		expect(text(fold)).toContain('Closed');
		expect(fold.getAttribute('aria-expanded')).toBe('false');
	});

	it('keeps a decision withdrawn in the default view on screen with its outcome, its group folded', async () => {
		net.queue = queue([entry('this_sprint', dec(1, 'Gate')), entry('open', dec(4, 'Free'))]);
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
		expect(row().closest('ul')!.id).toBe('decisions-this_sprint');
		// Counted where it is shown, though the server now puts it in Closed.
		expect(text(document.body.querySelector('h2'))).toBe('Needed this sprint 1');
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
		expect(document.body.textContent).toContain('Nothing holds work back');
		expect(headings()).toEqual(['Closed']);
	});

	it('renders no heading for an empty group', async () => {
		net.queue = queue([entry('this_sprint', dec(1, 'Gate')), entry('satisfied', dec(2, 'Settled', { record, check: accepted }))]);
		await open();
		expect(headings()).toEqual(['Needed this sprint', 'Accepted']);
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
			blockedStory({ key: 'RV-7', title: 'Seven' }),
			blockedStory({ key: 'RV-8', title: 'Eight' })
		];
		net.queue = queue([entry('this_sprint', dec(1, 'Gate'), blocks), entry('open', dec(4, 'Free'))]);
		await open();
		const lines = [...document.body.querySelectorAll('button[data-field="blocks"]')].map((l) => text(l));
		expect(lines).toEqual(['Holds 2 stories ▸']);
		expect(document.getElementById('decisions-this_sprint')!.textContent).toContain('Holds 2 stories');
	});

	it('says how fresh the newest record check is', async () => {
		net.queue = queue([
			entry('satisfied', dec(2, 'Settled', { record, check: accepted, checked_at: '2026-10-27T11:57:00Z' })),
			entry('this_sprint', dec(4, 'Proposed', { record, check: { kind: 'proposed_pr_open' }, checked_at: '2026-10-27T11:00:00Z' }))
		]);
		await open();
		expect(document.body.querySelector('.checked-all')?.textContent?.trim()).toBe('Records checked 3m ago');
	});

	it('says so when the queue is empty', async () => {
		await open();
		expect(document.body.textContent).toContain('Nothing holds work back');
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
			expect(document.body.querySelector('.notice.error')?.textContent).toContain('Could not load the readiness queue: boom');
		});
		expect([...document.body.querySelectorAll('.notice button')].map((b) => b.textContent)).toContain('Retry');
	});
});
