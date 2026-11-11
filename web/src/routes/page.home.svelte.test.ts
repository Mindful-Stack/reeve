// @vitest-environment jsdom
// Home: the decisions I decide (hidden for a non-decider, shown for a review count alone), Pick next with Start (moved,
// override prompt), the decision flag, a failed reload that keeps the page, and the live refetch.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { DecisionQueueEntry, StoryWithReadiness } from '#lib/api/types.ts';
import { session } from '#lib/me.svelte.ts';
import { decision, me, queueEntry, queueSummary, withReadiness } from '#lib/test/fixtures.ts';
import type { FakePage } from '#lib/test/net.ts';
import Home from './+page.svelte';

const live = vi.hoisted(() => ({ fns: [] as (() => unknown)[] }));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => {
		live.fns.push(fn);
	}
}));

const route = vi.hoisted(() => ({ fake: null as null | FakePage }));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	route.fake = fakePage({ url: '/' });
	return { page: route.fake.page };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn((to: string | URL) => route.fake!.goto(to)) }));

type Net = {
	gets: string[];
	posts: { path: string; body: unknown }[];
	entries: DecisionQueueEntry[];
	blocked: number;
	ready: StoryWithReadiness[];
	mine: StoryWithReadiness[];
	failQueue: boolean;
	move: 'ok' | 'override';
};
const net = vi.hoisted(() => ({ state: null as unknown as Net }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure, json } = await import('#lib/test/net.ts');
	installFetch(({ method, path, url, body }) => {
		const n = net.state;
		if (method === 'POST') {
			n.posts.push({ path, body });
			if (n.move === 'override') return json({ outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: true }, 422);
			const key = path.split('/')[4];
			const row = n.ready.find((r) => r.story.key === key)!;
			n.ready = n.ready.filter((r) => r !== row);
			const moved = { ...row.story, delivery: 'in_progress' as const, owner: 'me@example.com', version: 2 };
			n.mine = [...n.mine, { story: moved, readiness: row.readiness }];
			return moved;
		}
		n.gets.push(path);
		switch (path) {
			case '/api/v1/sprints':
				return { active: null, closed: [], draft: null };
			case '/api/v1/decisions/queue':
				if (n.failQueue) return failure('the queue is down');
				return { entries: n.entries, summary: queueSummary({ blocked_stories: n.blocked }) };
			case '/api/v1/stories':
				return url.searchParams.get('ready') === 'true' ? n.ready : n.mine;
		}
		throw new Error(`unexpected GET ${path}`);
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
beforeEach(() => {
	session.me = me();
	live.fns.length = 0;
	net.state = { gets: [], posts: [], entries: [], blocked: 0, ready: [], mine: [], failQueue: false, move: 'ok' };
});
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

async function open() {
	app = mount(Home, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.textContent).toContain('Pick next');
	});
}
const text = () => document.body.textContent ?? '';
const heading = (name: string) => [...document.body.querySelectorAll('h2')].find((h) => h.textContent === name);

describe('home', () => {
	it('hides Decisions you decide from someone who decides nothing', async () => {
		net.state.entries = [queueEntry({ lane: 'this_sprint', decision: decision({ id: 4, decider: 'other@example.com' }) })];
		await open();
		expect(heading('Decisions you decide')).toBeUndefined();
	});

	it('shows the section for a record to review alone, linking to the review lane', async () => {
		net.state.entries = [queueEntry({ lane: 'review', decision: decision({ id: 5, decider: 'me@example.com' }) })];
		await open();
		expect(heading('Decisions you decide')).toBeDefined();
		const review = [...document.body.querySelectorAll('a')].find((a) => a.textContent === '1 record to review →')!;
		expect(review.getAttribute('href')).toBe('/decisions?mine=1#review');
	});

	it('lists urgent work first in Pick next and Start makes an unowned story mine', async () => {
		net.state.ready = [withReadiness({ key: 'P-1', title: 'plain work' }), withReadiness({ key: 'U-1', title: 'urgent work', urgent_sources: [{ item: 'story', key: 'U-1' }] })];
		await open();
		const keys = [...document.body.querySelectorAll('section[aria-labelledby="home-pick-next"] li')].map((li) => li.getAttribute('data-key'));
		expect(keys).toEqual(['U-1', 'P-1']);
		const gets = net.state.gets.length;
		(document.body.querySelector('button[aria-label="Start U-1"]') as HTMLButtonElement).click();
		await vi.waitFor(() => {
			flushSync();
			expect(text()).toContain('U-1 is yours now.');
		});
		expect(net.state.posts).toEqual([{ path: '/api/v1/stories/U-1/move', body: { version: 1, to: 'in_progress' } }]);
		expect(net.state.gets.length).toBeGreaterThan(gets); // the page reloaded after the move
		const under = document.body.querySelector('section[aria-labelledby="home-under-way"]')!;
		expect(under.textContent).toContain('urgent work');
	});

	it('offers the override prompt when Start is refused for readiness', async () => {
		net.state.ready = [withReadiness({ key: 'U-1' })];
		net.state.move = 'override';
		await open();
		(document.body.querySelector('button[aria-label="Start U-1"]') as HTMLButtonElement).click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')?.textContent).toContain('U-1');
		});
		expect(text()).not.toContain('was not moved'); // not the notice an automation token would get
	});

	it('flags the stories that wait on a decision and links to what is not ready', async () => {
		net.state.blocked = 3;
		await open();
		expect(text()).toContain('3 stories wait on a decision.');
		const link = [...document.body.querySelectorAll('a')].find((a) => a.textContent === 'See what is not ready →')!;
		expect(link.getAttribute('href')).toBe('/board?notready=1');
	});

	it('keeps the page and says so when a live reload fails', async () => {
		net.state.ready = [withReadiness({ key: 'P-1', title: 'plain work' })];
		await open();
		net.state.failQueue = true;
		await live.fns[0]!();
		flushSync();
		expect(text()).toContain('Could not load Home');
		expect(text()).toContain('plain work');
	});

	it('refetches all four reads on a live entry', async () => {
		await open();
		net.state.gets = [];
		await live.fns[0]!();
		expect([...net.state.gets].sort()).toEqual(['/api/v1/decisions/queue', '/api/v1/sprints', '/api/v1/stories', '/api/v1/stories']);
	});
});
