// @vitest-environment jsdom
// The stale-response guard of every view that reloads: a reload that started earlier and answers later is dropped, so
// an older answer never overwrites a newer one. Each case holds the first load, starts a second through a live event,
// answers the second (NEWER) and then the first (OLDER), and asserts the view still shows NEWER. A view that lost its
// `seq !== latest` (or `generation`) check fails its case.
import { flushSync, mount, unmount, type Component } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { session } from '#lib/me.svelte.ts';
import StoryDrawer from '#lib/components/StoryDrawer.svelte';
import { activity, config, decision, epic, initiative, me, story, withReadiness } from '#lib/test/fixtures.ts';
import type { FakePage } from '#lib/test/net.ts';
import Planning from './+page.svelte';
import Activity from './(account)/activity/+page.svelte';
import Backlog from './backlog/+page.svelte';
import Board from './board/+page.svelte';
import Decisions from './decisions/+page.svelte';
import Epic from './epics/[key]/+page.svelte';
import Initiative from './initiatives/[key]/+page.svelte';

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
	route.fake = fakePage({ url: '/view', params: { key: 'K-1' } });
	return { page: route.fake.page };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn((to: string | URL) => route.fake!.goto(to)) }));

// Every GET is held until the test answers it with a mark; `auto` answers new requests at once.
type Pending = { path: string; answer: (mark: string) => void };
const held = vi.hoisted(() => ({
	pending: [] as Pending[],
	auto: null as string | null,
	reply: (_path: string, _query: URLSearchParams, _mark: string): unknown => {
		throw new Error('no reply set');
	}
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(({ method, path, url }) => {
		if (method !== 'GET') throw new Error(`unexpected ${method} ${path}`);
		return new Promise((resolve) => {
			const answer = (mark: string) => resolve(held.reply(path, url.searchParams, mark));
			if (held.auto) answer(held.auto);
			else held.pending.push({ path, answer });
		});
	});
	return orig();
});

/** A valid answer for every GET the views make; every title (and the activity entry's key) carries `mark`. */
held.reply = (path, query, mark) => {
	const i = initiative({ key: 'K-1', title: `${mark} initiative` });
	const e = epic({ key: 'K-1', title: `${mark} epic`, initiative: 'K-1' });
	const s = story({ key: 'K-1', title: `${mark} story`, epic: 'K-1' });
	switch (path) {
		case '/api/v1/initiatives':
			return [i];
		case '/api/v1/initiatives/K-1':
			return i;
		case '/api/v1/epics':
			return [e];
		case '/api/v1/epics/K-1':
			return e;
		case '/api/v1/stories':
			return [withReadiness(s)];
		case '/api/v1/stories/K-1':
			return s;
		case '/api/v1/stories/K-1/ready':
			return { ready: true, reasons: [] };
		case '/api/v1/stories/K-1/comments':
		case '/api/v1/items/K-1/comments':
		case '/api/v1/epics/K-1/findings':
		case '/api/v1/initiatives/K-1/findings':
			return [];
		case '/api/v1/decisions':
			// The item pages and the drawer read the plain list.
			return [decision({ on: 'K-1', title: `${mark} decision`, decider: me().email })];
		case '/api/v1/sprints':
			return { active: null, closed: [], draft: null };
		case '/api/v1/decisions/queue':
			// The decisions page reads the queue.
			return {
				entries: [{ group: 'open', decision: decision({ on: 'K-1', title: `${mark} decision` }), blocks: [] }],
				summary: { open: 1, blocking: 0, blocked_stories: 0, overdue: 0 }
			};
		case '/api/v1/activity':
			// An item's own log is empty; the activity page reads the whole log, whose entries it renders by key.
			return query.has('entity_key') ? [] : [activity({ entity_key: `${mark}-1` })];
	}
	throw new Error(`unexpected GET ${path}`);
};

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	live.fns.length = 0;
	held.pending.length = 0;
	held.auto = null;
	route.fake!.reset();
});

/** Lets every request a round starts reach `fetch` (openapi-fetch awaits before calling it). */
const settle = () => new Promise((r) => setTimeout(r, 20));

async function round(): Promise<Pending[]> {
	await vi.waitFor(() => expect(held.pending.length).toBeGreaterThan(0));
	await settle();
	return held.pending.splice(0);
}

/** What the page shows: its text and every input's value (the drawer shows the story's title in an input). */
function shown(): string {
	return document.body.textContent + ' ' + [...document.body.querySelectorAll('input')].map((i) => i.value).join(' ');
}

/** What the drawer shows outside its epic select and epic links: they name epics from `GET /epics`, which the drawer
 *  loads once on mount and outside the guard, so they carry the first round's mark. */
function drawerShown(): string {
	const copy = document.body.cloneNode(true) as HTMLElement;
	for (const s of copy.querySelectorAll('select, .epic-link')) s.remove();
	return copy.textContent + ' ' + [...document.body.querySelectorAll('input')].map((i) => i.value).join(' ');
}

/** Mounts the view, races two reloads and asserts that the older one's late answer is dropped. */
async function keepsNewest(view: Component<never>, props: Record<string, unknown> = {}, text = shown) {
	session.config = config({ horizon_buckets: ['Now'] });
	session.me = me();
	app = mount(view as Component<Record<string, unknown>>, { target: document.body, props });
	const first = await round();
	expect(live.fns.length).toBeGreaterThan(0);
	for (const f of live.fns) void f();
	const second = await round();

	// Requests a round starts after it is answered (an item's advisory reads, the drawer's activity read) get its mark.
	held.auto = 'NEWER';
	for (const r of second) r.answer('NEWER');
	await vi.waitFor(() => {
		flushSync();
		expect(text()).toContain('NEWER');
	});

	held.auto = 'OLDER';
	for (const r of first) r.answer('OLDER');
	await settle();
	flushSync();
	expect(text()).toContain('NEWER');
	expect(text()).not.toContain('OLDER');
}

describe('a late answer to an older reload is dropped', () => {
	it('on the planning page', () => keepsNewest(Planning));
	it('on the backlog', () => keepsNewest(Backlog));
	it('on an epic page', () => keepsNewest(Epic));
	it('on an initiative page', () => keepsNewest(Initiative));
	it('on the decisions page', () => keepsNewest(Decisions));
	it('on the board', () => keepsNewest(Board));
	it('on the activity page', () => keepsNewest(Activity));
	it('in the story drawer', () => keepsNewest(StoryDrawer, { storyKey: 'K-1', onclose: () => {} }, drawerShown));
});
