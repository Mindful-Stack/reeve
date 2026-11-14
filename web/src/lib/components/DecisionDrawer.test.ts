// @vitest-environment jsdom
// The decision drawer reads GET /decisions/{id}/context: when the decision is needed, what it needs next, its record, the
// stories it holds, and (through DecisionList, with its discussion open) the decision itself.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DecisionQueueEntry } from '#lib/api/types.ts';
import { resetDecisionDraftsForTests } from '#lib/decisionDrafts.svelte.ts';
import { today } from '#lib/decisions.ts';
import { blockedStory, decision, queueEntry } from '#lib/test/fixtures.ts';
import { failure, settle } from '#lib/test/net.ts';
import DecisionDrawer from './DecisionDrawer.svelte';

vi.mock('$app/state', async () => ({
	page: (await import('#lib/test/net.ts')).fakePage({ url: '/board?decision=7&sort=rank' }).page
}));

const live = vi.hoisted(() => ({ onLive: [] as (() => unknown)[], poll: [] as (() => unknown)[] }));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => void live.onLive.push(fn),
	everyVisible: (fn: () => unknown) => void live.poll.push(fn)
}));

const net = vi.hoisted(() => ({
	/** Every request's method and path, in order. */
	sent: [] as { method: string; path: string }[],
	/** What the context read answers: an entry, or a Response (a failure). */
	context: null as unknown
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(({ method, path }) => {
		net.sent.push({ method, path });
		if (method === 'GET' && path === '/api/v1/decisions/7/context') return net.context;
		if (method === 'GET' && path.endsWith('/comments')) return [];
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

const contexts = () => net.sent.filter((c) => c.path === '/api/v1/decisions/7/context').length;

const RECORD = { repo: 'kb', path: 'docs/adr/7.md', record_id: 'kb/ADR-7' };

function entry(over: Partial<DecisionQueueEntry> = {}): DecisionQueueEntry {
	return queueEntry({
		decision: decision({ id: 7, on: 'ABC-1', on_type: 'story', title: 'Pick a queue', decider: 'dan@example.com', due: today() }),
		lane: 'this_sprint',
		needed_in: 'this_sprint',
		needed_in_reason: { kind: 'goal', goal_id: 3, outcome: 'Ship tiers' },
		next_step: { kind: 'write_record' },
		...over
	});
}

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.sent.length = 0;
	net.context = null;
	live.onLive.length = 0;
	live.poll.length = 0;
	resetDecisionDraftsForTests();
});

async function open(e: unknown = entry(), onclose: () => void = () => {}): Promise<HTMLElement> {
	net.context = e;
	app = mount(DecisionDrawer, { target: document.body, props: { decisionId: 7, onclose } });
	return await shown();
}

/** The drawer once its first answer is rendered (or its failure). */
async function shown(): Promise<HTMLElement> {
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('aside')?.textContent).not.toContain('Loading');
	});
	return document.body.querySelector('aside')!;
}

const words = (el: Element | null | undefined) => el?.textContent?.replace(/\s+/g, ' ').trim();

describe('DecisionDrawer', () => {
	it('reads the decision context and shows the decision, the item link, when it is needed and the next step', async () => {
		const drawer = await open();
		expect(net.sent[0]).toEqual({ method: 'GET', path: '/api/v1/decisions/7/context' });
		expect(drawer.getAttribute('aria-label')).toBe('Decision #7');
		// The heading carries the decision's title with its number muted beside it.
		expect(words(drawer.querySelector('h2'))).toBe('Pick a queue #7');
		expect(words(drawer.querySelector('.decisions .title'))).toBe('Pick a queue');
		// A story item opens its drawer over this view, which drops the decision parameter.
		expect(drawer.querySelector('[data-testid="item-link"]')?.getAttribute('href')).toBe('/board?sort=rank&story=ABC-1');
		const strip = drawer.querySelector('[data-testid="strip"]')!;
		expect(words(strip)).toBe('Needed this sprint · for goal: Ship tiers Write a record, or close it dan decides · Due today');
		// The row is not repeated: no second title, and no holds line from the list (the strip carries the next step).
		expect(drawer.querySelectorAll('.title').length).toBe(1);
		expect(drawer.querySelector('.decisions [data-field="blocks"]')).toBeNull();
		expect(drawer.querySelector('.decisions .next-step')).toBeNull();
	});

	it('puts the strip, the record and the held work above the decision row, so a long thread cannot push them off-screen', async () => {
		const drawer = await open(
			entry({ decision: decision({ id: 7, record: RECORD }), blocks: [blockedStory({ key: 'ABC-31', needed_in: 'this_sprint' })] })
		);
		const order = ['[data-testid="strip"]', '[data-testid="record"]', '.held', '.decision-list'].map((sel) => {
			const el = drawer.querySelector(sel);
			expect(el, sel).not.toBeNull();
			return el!;
		});
		for (let i = 1; i < order.length; i++) {
			expect(order[i - 1].compareDocumentPosition(order[i]) & Node.DOCUMENT_POSITION_FOLLOWING, `${i}`).toBeTruthy();
		}
	});

	it('says an open decision nothing waits on is not needed yet, and that it holds nothing yet', async () => {
		const drawer = await open(entry({ needed_in: 'none', needed_in_reason: null, next_step: null }));
		expect(words(drawer.querySelector('[data-testid="strip"]'))).toBe('Not needed yet dan decides · Due today');
		const held = drawer.querySelector('.held')!;
		expect(words(held)).toBe('Held work Holds nothing yet');
		expect(held.getAttribute('aria-labelledby')).toBe(held.querySelector('h3')!.id);
		expect(held.querySelector('ul')).toBeNull();
		// A Now decision says its Now line in place of the needed label; a nested one adds the row it is under.
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		const now = await open(
			entry({ lane: 'beyond', needed_in: 'beyond', needed_in_reason: { kind: 'held' }, blocks: [blockedStory({ key: 'ABC-31' }), blockedStory({ key: 'ABC-32' })] })
		);
		expect(words(now.querySelector('[data-testid="strip"] .needed'))).toBe('Now \u00b7 holds 2');
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		const nested = await open(entry({ under: 'ABC-1' }));
		expect(words(nested.querySelector('[data-testid="strip"] .needed'))).toBe('Needed this sprint \u00b7 for goal: Ship tiers \u00b7 under ABC-1, in discovery');
	});

	it('says a closed or accepted decision holds nothing, without "yet"', async () => {
		for (const decisionOver of [
			{ closed: { at: '2026-11-01T00:00:00Z', by: 'dan@example.com', reason: 'moot', kind: 'withdrawn' as const } },
			{ record: RECORD, check: { kind: 'accepted' as const, commit: 'abcdef0123' } }
		]) {
			const drawer = await open(entry({ decision: decision({ id: 7, ...decisionOver }), next_step: null, needed_in: 'none', needed_in_reason: null }));
			expect(words(drawer.querySelector('.held'))).toBe('Held work Holds nothing');
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});

	it('does not say it holds nothing when only stories under way remain', async () => {
		const drawer = await open(entry({ under_way: [blockedStory({ key: 'ABC-34', delivery: 'in_progress' })] }));
		expect(drawer.querySelector('.held')?.textContent).not.toContain('Holds nothing yet');
		expect(drawer.querySelector('[data-group="under-way"]')).not.toBeNull();
	});

	it('folds a held story with no window into Beyond', async () => {
		const drawer = await open(entry({ blocks: [blockedStory({ key: 'ABC-31', needed_in: 'none' }), blockedStory({ key: 'ABC-32', needed_in: 'beyond' })] }));
		const groups = [...drawer.querySelectorAll<HTMLElement>('.held .stories')].map((ul) => [ul.dataset.group, ul.querySelectorAll('li').length]);
		expect(groups).toEqual([['beyond', 2]]);
		expect(drawer.querySelector('.held')?.textContent).not.toContain('Not scheduled');
	});

	it('opens an initiative and an epic on their pages', async () => {
		for (const [on_type, href] of [
			['initiative', '/initiatives/ABC-1'],
			['epic', '/epics/ABC-1']
		] as const) {
			const drawer = await open(entry({ decision: decision({ id: 7, on: 'ABC-1', on_type }) }));
			expect(drawer.querySelector('[data-testid="item-link"]')?.getAttribute('href')).toBe(href);
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});

	it('lists the held stories by when they are needed, each as a link to its drawer, then the ones under way', async () => {
		const drawer = await open(
			entry({
				blocks: [
					blockedStory({ key: 'ABC-31', title: 'Tier page', needed_in: 'this_sprint', owner: 'ella@example.com', urgent: true }),
					blockedStory({ key: 'ABC-32', title: 'Tier pricing', needed_in: 'next_sprint' }),
					blockedStory({ key: 'ABC-33', title: 'Tier billing', needed_in: 'beyond' })
				],
				under_way: [blockedStory({ key: 'ABC-34', title: 'Tier emails', delivery: 'in_progress', needed_in: 'this_sprint' })]
			})
		);
		const groups = [...drawer.querySelectorAll<HTMLElement>('.held .stories')].map((ul) => [
			ul.dataset.group,
			ul.previousElementSibling?.textContent,
			[...ul.querySelectorAll<HTMLAnchorElement>('a.key')].map((a) => a.getAttribute('href'))
		]);
		expect(groups).toEqual([
			['this_sprint', 'This sprint', ['/board?sort=rank&story=ABC-31']],
			['next_sprint', 'Next sprint', ['/board?sort=rank&story=ABC-32']],
			['beyond', 'Beyond', ['/board?sort=rank&story=ABC-33']],
			['under-way', 'Under way without it', ['/board?sort=rank&story=ABC-34']]
		]);
		for (const a of drawer.querySelectorAll('.held a.key')) expect(a.getAttribute('href')).not.toContain('decision');
		for (const a of drawer.querySelectorAll('.held a.key')) expect(a.getAttribute('data-sveltekit-reset')).toBe('false');
		expect(drawer.querySelector('.held h3:last-of-type')?.textContent).toBe('Under way without it');
		// A row: kind glyph, key link, title, the urgent chip and the owner; an under-way story also says where it is.
		const first = drawer.querySelector('.held li')!;
		expect(first.querySelector('.glyph')).not.toBeNull();
		expect(words(first)).toContain('ABC-31 Tier page');
		expect(first.querySelector('.urgent-chip')).not.toBeNull();
		expect(words(first)).toContain('ella');
		const wayRow = drawer.querySelector('[data-group="under-way"] li')!;
		expect(words(wayRow)).toContain('ABC-34 Tier emails');
		expect(words(wayRow)).toContain('in progress');
		expect(wayRow.querySelector('.urgent-chip')).toBeNull();
	});

	it('links the record through browse_url, or names it when there is none, and links the PR once, in the strip, when the record awaits review', async () => {
		const withUrl = { ...RECORD, browse_url: 'https://github.com/o/kb/blob/HEAD/docs/adr/7.md' };
		let drawer = await open(
			entry({
				decision: decision({ id: 7, record: withUrl, pr_url: 'https://github.com/o/kb/pull/3', check: { kind: 'proposed_pr_open' } }),
				next_step: { kind: 'review_record', pr_url: 'https://github.com/o/kb/pull/3' }
			})
		);
		const block = drawer.querySelector('[data-testid="record"]')!;
		expect([...block.querySelectorAll('a')].map((a) => [a.textContent, a.getAttribute('href')])).toEqual([['kb/docs/adr/7.md', withUrl.browse_url]]);
		expect(drawer.querySelector('.strip .next-step a')?.getAttribute('href')).toBe('https://github.com/o/kb/pull/3');
		expect([...drawer.querySelectorAll('[data-testid="strip"] a, [data-testid="record"] a')].filter((a) => a.getAttribute('href') === 'https://github.com/o/kb/pull/3')).toHaveLength(1);
		unmount(app!);
		app = null;
		document.body.innerHTML = '';

		drawer = await open(entry({ decision: decision({ id: 7, record: RECORD }) }));
		const plain = drawer.querySelector('[data-testid="record"]')!;
		expect(plain.querySelector('a')).toBeNull();
		expect(words(plain)).toBe('Record kb/docs/adr/7.md');
	});

	it('keeps the linked PR in the record block when the next step is not a review', async () => {
		const drawer = await open(
			entry({
				decision: decision({ id: 7, record: RECORD, pr_url: 'https://github.com/o/kb/pull/3', check: { kind: 'not_found' } }),
				next_step: { kind: 'fix_record', result: { kind: 'not_found' } }
			})
		);
		expect(words(drawer.querySelector('.strip .next-step'))).toBe('Fix the record: not found');
		expect([...drawer.querySelectorAll('[data-testid="record"] a')].map((a) => [a.textContent, a.getAttribute('href')])).toEqual([
			['Linked PR', 'https://github.com/o/kb/pull/3']
		]);
	});

	it('names the decider, the date line and the reversibility in the strip', async () => {
		let drawer = await open(entry({ decision: decision({ id: 7, decider: 'dan@example.com', due: today(), reversibility: 'one_way' }) }));
		expect(words(drawer.querySelector('[data-testid="who"]'))).toBe('dan decides · Due today · One-way');
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		// No due date and no reversibility: the date line says how long it has been open, and the last part is left out.
		drawer = await open(entry({ decision: decision({ id: 7, decider: 'dan@example.com', due: null, created_at: new Date().toISOString() }) }));
		expect(words(drawer.querySelector('[data-testid="who"]'))).toBe('dan decides · No date · open 0 days');
	});

	it('names the promised item: a story opens its drawer over this view, an epic its page', async () => {
		for (const [level, key, href] of [
			['story', 'ABC-12', '/board?sort=rank&story=ABC-12'],
			['epic', 'ABC-5', '/epics/ABC-5']
		] as const) {
			const drawer = await open(
				entry({ needed_in: 'this_sprint', needed_in_reason: { kind: 'promise', key, level, on: '2026-11-10', overdue: true } })
			);
			const a = drawer.querySelector<HTMLAnchorElement>('.strip .needed a');
			expect(a?.getAttribute('href')).toBe(href);
			// The view behind the story drawer must not move (drawer.ts: storyHref); a page link navigates normally.
			expect(a?.getAttribute('data-sveltekit-reset')).toBe(level === 'story' ? 'false' : null);
			expect(words(drawer.querySelector('.strip .needed'))).toBe(`Needed this sprint · promised by 10 Nov, overdue: ${key}`);
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});

	it('links the item in discovery to its page, by its level', async () => {
		for (const [level, href] of [
			['initiative', '/initiatives/GRA-12'],
			['epic', '/epics/GRA-12']
		] as const) {
			const drawer = await open(
				entry({ needed_in: 'next_sprint', needed_in_reason: { kind: 'discovery', key: 'GRA-12', level } })
			);
			const a = drawer.querySelector<HTMLAnchorElement>('.strip .needed a');
			expect(a?.getAttribute('href')).toBe(href);
			expect(words(drawer.querySelector('.strip .needed'))).toBe('Needed next sprint · GRA-12 is in discovery');
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});

	it('says just Closed for a closure with no kind (before kb/ADR-0013)', async () => {
		const drawer = await open(
			entry({
				decision: decision({ id: 7, closed: { at: '2026-11-01T00:00:00Z', by: 'dan@example.com', reason: 'moot', kind: null } }),
				next_step: null,
				needed_in: 'none',
				needed_in_reason: null
			})
		);
		const lines = [...drawer.querySelectorAll('p')].map(words);
		expect(lines).toContain('Closed');
		expect(drawer.textContent).not.toContain('null');
	});

	it('says satisfied by its record and closed as separate lines', async () => {
		const drawer = await open(
			entry({
				decision: decision({
					id: 7,
					record: RECORD,
					check: { kind: 'accepted', commit: 'abcdef0123' },
					closed: { at: '2026-11-01T00:00:00Z', by: 'dan@example.com', reason: 'moot', kind: 'decided' }
				}),
				next_step: null,
				needed_in: 'none',
				needed_in_reason: null
			})
		);
		const lines = [...drawer.querySelectorAll('[data-testid="record"] p')].map(words);
		expect(lines).toContain('Satisfied by its record (abcdef0)');
		expect(lines).toContain('Closed · decided');
		expect(drawer.querySelector('[data-testid="strip"]')).toBeNull();
	});

	it('shows the mismatch warning when the decision and its record differ in reversibility', async () => {
		const drawer = await open(
			entry({
				decision: decision({
					id: 7,
					reversibility: 'one_way',
					record: RECORD,
					check: { kind: 'not_accepted', commit: 'abcdef0123', status: 'proposed', reversibility: 'two_way' }
				})
			})
		);
		expect(drawer.querySelector('.mismatch')?.textContent).toContain('record says two-way');
	});

	it('opens the discussion and loads its thread', async () => {
		const drawer = await open();
		await vi.waitFor(() => expect(net.sent).toContainEqual({ method: 'GET', path: '/api/v1/decisions/7/comments' }));
		flushSync();
		expect(drawer.querySelector('button[data-field="discussion"]')?.getAttribute('aria-expanded')).toBe('true');
		expect(drawer.querySelector('textarea[data-field="comment"]')).not.toBeNull();
	});

	it('says there is no such decision on a 404', async () => {
		const drawer = await open(failure('no such decision', 404, 'not_found'));
		expect(words(drawer.querySelector('[data-testid="missing"]'))).toBe('No decision #7');
		expect(drawer.querySelector('.decisions')).toBeNull();
	});

	it('says why it could not load, when the first read fails', async () => {
		const drawer = await open(failure('boom'));
		expect(drawer.querySelector('[role="alert"]')?.textContent).toContain('Could not load decision #7: boom');
	});

	it('keeps its content and offers a retry when a refetch fails after a load', async () => {
		const drawer = await open();
		net.context = failure('boom');
		void live.onLive[0]();
		await vi.waitFor(() => {
			flushSync();
			expect(drawer.querySelector('[role="status"]')?.textContent).toContain('Could not reload decision #7: boom');
		});
		expect(words(drawer.querySelector('.decisions .title'))).toBe('Pick a queue');
		net.context = entry({ decision: decision({ id: 7, title: 'Pick a better queue' }) });
		[...drawer.querySelectorAll<HTMLButtonElement>('[role="status"] button')].find((b) => b.textContent === 'Retry')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(words(drawer.querySelector('.decisions .title'))).toBe('Pick a better queue');
		});
		expect(drawer.querySelector('[role="status"]')).toBeNull();
	});
});

describe('DecisionDrawer refresh', () => {
	it('refetches on a live event', async () => {
		await open();
		expect(contexts()).toBe(1);
		net.context = entry({ decision: decision({ id: 7, title: 'Renamed' }) });
		for (const f of live.onLive) void f();
		await vi.waitFor(() => expect(contexts()).toBeGreaterThanOrEqual(2));
		await vi.waitFor(() => {
			flushSync();
			expect(words(document.body.querySelector('.decisions .title'))).toBe('Renamed');
		});
	});

	it('polls only while a record is linked and the decision is open', async () => {
		const poll = () => {
			for (const f of live.poll) void f();
		};
		// No record: no poll.
		await open();
		poll();
		await settle();
		flushSync();
		expect(contexts()).toBe(1);
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		live.onLive.length = live.poll.length = 0;
		net.sent.length = 0;

		// A record on an open decision: polls.
		await open(entry({ decision: decision({ id: 7, record: RECORD }) }));
		poll();
		await vi.waitFor(() => expect(contexts()).toBe(2));
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		live.onLive.length = live.poll.length = 0;
		net.sent.length = 0;

		// A closed decision: no poll.
		await open(
			entry({ decision: decision({ id: 7, record: RECORD, closed: { at: '2026-11-01T00:00:00Z', by: 'dan@example.com', reason: 'moot', kind: 'withdrawn' } }) })
		);
		poll();
		await settle();
		flushSync();
		expect(contexts()).toBe(1);
	});

	it('renders a decision closed on refetch as closed', async () => {
		const drawer = await open(entry({ decision: decision({ id: 7, record: RECORD }) }));
		expect(drawer.querySelector('.decisions li.closed')).toBeNull();
		net.context = entry({
			decision: decision({ id: 7, record: RECORD, closed: { at: '2026-11-01T00:00:00Z', by: 'dan@example.com', reason: 'no longer needed', kind: 'withdrawn' } }),
			next_step: null
		});
		for (const f of live.onLive) void f();
		await vi.waitFor(() => {
			flushSync();
			expect(drawer.querySelector('.decisions li.closed')).not.toBeNull();
		});
		expect(drawer.textContent).toContain('Withdrawn by dan: no longer needed');
	});
});

describe('DecisionDrawer chrome', () => {
	it('focuses its heading once the first read resolves', async () => {
		const drawer = await open();
		await vi.waitFor(() => expect(document.activeElement).toBe(drawer.querySelector('h2')));
		expect(drawer.querySelector('h2')?.getAttribute('tabindex')).toBe('-1');
	});

	it('closes on Escape, and on the close button', async () => {
		const onclose = vi.fn();
		const drawer = await open(entry(), onclose);
		drawer.querySelector('h2')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		expect(onclose).toHaveBeenCalledTimes(1);
		drawer.querySelector<HTMLButtonElement>('button.close')!.click();
		expect(onclose).toHaveBeenCalledTimes(2);
	});

	it('leaves Escape to a form field and to a handler that already took it', async () => {
		const onclose = vi.fn();
		const drawer = await open(entry(), onclose);
		await vi.waitFor(() => expect(drawer.querySelector('textarea')).not.toBeNull());
		drawer.querySelector('textarea')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		const taken = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
		drawer.querySelector('.decisions')!.addEventListener('keydown', (ev) => ev.preventDefault(), { once: true });
		drawer.querySelector('.decisions')!.dispatchEvent(taken);
		expect(onclose).not.toHaveBeenCalled();
	});
});
