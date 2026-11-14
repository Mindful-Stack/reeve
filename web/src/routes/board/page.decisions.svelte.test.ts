// @vitest-environment jsdom
// The board's decision pieces: a banner of what needs a decision (the queue's summary), the Not ready summary saying how
// many cards wait on decisions, and the queue as an extra: its failure never blocks the cards (kb/ADR-0018 R1).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DecisionQueue, Reason, StoryWithReadiness } from '#lib/api/types.ts';
import { queueSummary, withReadiness } from '#lib/test/fixtures.ts';
import { failure, settle, type Call } from '#lib/test/net.ts';
import Page from './+page.svelte';

vi.mock('$app/state', async () => {
	const { SvelteURL } = await import('svelte/reactivity');
	return { page: { url: new SvelteURL('http://localhost/board') } };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('#lib/me.svelte.ts', async () => ({
	session: await (async () => {
		const { config, me } = await import('#lib/test/fixtures.ts');
		return { me: me({ email: 'me@example.com' }), config: config({ done_window_days: 14 }) };
	})(),
	isInteractive: () => true,
	isAdmin: () => false,
	load: () => Promise.resolve()
}));
const live = vi.hoisted(() => ({ fns: [] as (() => void)[] }));
vi.mock('#lib/events.svelte.ts', () => ({
	live: { connected: false, lastId: 0, version: 0, latest: null, recent: [] },
	onLive: (fn: () => void) => live.fns.push(fn),
	freshNote: () => null
}));

const net = vi.hoisted(() => ({
	paths: [] as string[],
	handler: (_c: import('#lib/test/net.ts').Call): unknown => null
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch((c) => {
		net.paths.push(c.path);
		return net.handler(c);
	});
	return orig();
});

const decisionReason = (id: number): Reason => ({ reason: 'decision_unchecked', decision_id: id, title: `Decision ${id}` });
let stories: StoryWithReadiness[];
/** What the queue answers: the summary, or null for a server error. */
let queue: DecisionQueue | null;

function serve(c: Call): unknown {
	if (c.path === '/api/v1/stories') return stories;
	if (c.path === '/api/v1/epics') return [];
	if (c.path === '/api/v1/sprints') return { active: null, closed: [], draft: null };
	if (c.path === '/api/v1/decisions/queue') return queue ?? failure('queue is down');
	if (c.method === 'POST' && c.path === '/api/v1/stories/ABC-1/move') {
		stories = [withReadiness({ key: 'ABC-1', title: 'Ready one', delivery: 'in_progress', version: 2 }), ...stories.slice(1)];
		return stories[0].story;
	}
	throw new Error(`unexpected ${c.method} ${c.path}`);
}

beforeEach(() => {
	stories = [
		withReadiness({ key: 'ABC-1', title: 'Ready one' }),
		withReadiness({ key: 'ABC-2', title: 'Waits on a decision' }, [decisionReason(7)]),
		withReadiness({ key: 'ABC-3', title: 'No criteria' }, [{ reason: 'no_criteria' }])
	];
	queue = { entries: [], discoveries: [], summary: queueSummary({ review: 1, this_sprint: 2 }) };
	net.handler = serve;
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.paths.length = 0;
	live.fns.length = 0;
});

const q = <T extends Element = HTMLElement>(sel: string) => document.body.querySelector<T>(sel);
const text = (el: Element | null | undefined) => el?.textContent?.replace(/\s+/g, ' ').trim();
async function mounted() {
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(q('[data-testid="card-ABC-1"]')).not.toBeNull();
	});
}

describe('board decisions', () => {
	it('fetches the queue and shows the banner above the columns', async () => {
		await mounted();
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('nav[aria-label="Readiness needing attention"]'))).toBe('⚑ 2 decisions needed this sprint');
		});
		expect(net.paths).toContain('/api/v1/decisions/queue');
		const banner = q('nav.decision-banner')!;
		expect(banner.compareDocumentPosition(q('.board')!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
	});

	it('shows no banner when nothing needs attention, once the answer has landed', async () => {
		// Fail first, so the Retry note proves the answer is awaited; its disappearance proves the zero answer landed.
		queue = null;
		await mounted();
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('.queue-note'))).toBe('Could not load the readiness queue · Retry');
		});
		queue = { entries: [], discoveries: [], summary: queueSummary() };
		[...document.body.querySelectorAll<HTMLButtonElement>('.queue-note button')].find((b) => b.textContent === 'Retry')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(q('.queue-note')).toBeNull();
		});
		expect(q('nav.decision-banner')).toBeNull();
	});

	it('says the counts may be out of date even when the last good summary had no parts, and Retry brings the banner', async () => {
		queue = { entries: [], discoveries: [], summary: queueSummary() };
		await mounted();
		await vi.waitFor(() => expect(net.paths.filter((p) => p === '/api/v1/decisions/queue')).toHaveLength(1));
		await settle();
		flushSync();
		expect(q('.queue-note')).toBeNull();
		queue = null;
		live.fns.forEach((f) => f());
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('.queue-note'))).toBe('Readiness queue counts may be out of date · Retry');
		});
		expect(q('nav.decision-banner')).toBeNull();
		queue = { entries: [], discoveries: [], summary: queueSummary({ this_sprint: 2 }) };
		[...document.body.querySelectorAll<HTMLButtonElement>('.queue-note button')].find((b) => b.textContent === 'Retry')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('nav.decision-banner'))).toBe('⚑ 2 decisions needed this sprint');
		});
		expect(q('.queue-note')).toBeNull();
	});

	it("reloads the cards but not the decisions summary after the user's own card move; a live event reloads both", async () => {
		await mounted();
		const queueReads = () => net.paths.filter((p) => p === '/api/v1/decisions/queue').length;
		const storyReads = () => net.paths.filter((p) => p === '/api/v1/stories').length;
		await vi.waitFor(() => expect(queueReads()).toBe(1));
		const before = storyReads();
		q<HTMLButtonElement>('[data-testid="column-todo"] button[aria-label="Actions for ABC-1"]')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(q('[role="menu"]')).not.toBeNull();
		});
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === 'Move to in progress')!.click();
		await vi.waitFor(() => expect(storyReads()).toBe(before + 1));
		await vi.waitFor(() => {
			flushSync();
			expect(q('[data-testid="column-in_progress"] [data-testid="card-ABC-1"]')).not.toBeNull();
		});
		expect(queueReads()).toBe(1);
		for (const f of live.fns) f();
		await vi.waitFor(() => expect(queueReads()).toBe(2));
	});

	it('says how many not-ready cards wait on decisions', async () => {
		await mounted();
		expect(text(q('details.not-ready > summary'))).toBe('Not ready · 2 · 1 waits on decisions');
	});

	it('uses the plural when several wait, and says nothing when none do', async () => {
		stories = [
			withReadiness({ key: 'ABC-1', title: 'A' }, [decisionReason(7)]),
			withReadiness({ key: 'ABC-2', title: 'B' }, [decisionReason(8)]),
			withReadiness({ key: 'ABC-3', title: 'C' }, [{ reason: 'no_criteria' }])
		];
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('details.not-ready > summary'))).toBe('Not ready · 3 · 2 wait on decisions');
		});
		unmount(app);
		document.body.innerHTML = '';
		stories = [withReadiness({ key: 'ABC-1' }), withReadiness({ key: 'ABC-3' }, [{ reason: 'no_criteria' }])];
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('details.not-ready > summary'))).toBe('Not ready · 1');
		});
	});

	it('keeps the cards when the first queue load fails, and offers a retry in the banner place', async () => {
		queue = null;
		await mounted();
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('.queue-note'))).toBe('Could not load the readiness queue · Retry');
		});
		expect(q('nav.decision-banner')).toBeNull();
		expect(q('[data-testid="card-ABC-2"]')).not.toBeNull();
		queue = { entries: [], discoveries: [], summary: queueSummary({ next_sprint: 1 }) };
		[...document.body.querySelectorAll<HTMLButtonElement>('.queue-note button')].find((b) => b.textContent === 'Retry')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('nav.decision-banner'))).toBe('⚑ 1 decision needed next sprint');
		});
		expect(q('.queue-note')).toBeNull();
	});

	it('keeps the last good summary with a note when a refetch fails, and clears the note on success', async () => {
		await mounted();
		await vi.waitFor(() => expect(q('nav.decision-banner')).not.toBeNull());
		queue = null;
		live.fns.forEach((f) => f());
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('.queue-note'))).toBe('Readiness queue counts may be out of date · Retry');
		});
		expect(text(q('nav.decision-banner'))).toBe('⚑ 2 decisions needed this sprint');
		expect(q('[data-testid="card-ABC-1"]')).not.toBeNull();
		queue = { entries: [], discoveries: [], summary: queueSummary({ next_sprint: 3 }) };
		live.fns.forEach((f) => f());
		await vi.waitFor(() => {
			flushSync();
			expect(text(q('nav.decision-banner'))).toBe('⚑ 3 decisions needed next sprint');
		});
		expect(q('.queue-note')).toBeNull();
	});
});
