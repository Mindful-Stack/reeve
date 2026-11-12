// @vitest-environment jsdom
// web/src/routes/triage/page.svelte.test.ts
// /triage: the untriaged queue oldest first, one item at a time, placed or dropped from the keyboard; drafts, Undo,
// conflicts and live changes. The fake server applies writes as the real one does (version bump, 409 on a stale version,
// placed and dropped items leave the untriaged list).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Comment, Epic, SimilarStory, Story } from '#lib/api/types.ts';
import { epic, story, withReadiness } from '#lib/test/fixtures.ts';
import { settle, type FakePage } from '#lib/test/net.ts';
import { goto } from '$app/navigation';
import Triage from './+page.svelte';

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
	route.fake = fakePage({ url: '/triage' });
	return { page: route.fake.page };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn((to: string | URL) => route.fake!.goto(to)) }));

/** Every field any write of this page sends: a PATCH's fields, a move's, a comment's. */
type Sent = Partial<Pick<Story, 'epic' | 'kind' | 'title' | 'criteria'>> & {
	version?: number;
	to?: Story['delivery'];
	resolution?: Story['resolution'];
	body?: string;
	idempotency_key?: string;
};
type Write = { method: string; path: string; body: Sent };
type Net = {
	stories: Story[];
	epics: Epic[];
	similar: Record<string, SimilarStory[]>;
	comments: Record<string, { body?: string; key?: string }[]>;
	writes: Write[];
	gets: string[];
	/** Paths whose answers wait for a call in `held`. */
	hold: Set<string>;
	held: (() => void)[];
	failComment: boolean;
};
const net = vi.hoisted(() => ({ s: null as unknown as Net }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, json, failure } = await import('#lib/test/net.ts');
	const conflict = (current: number) => json({ outcome: 'error', code: 'conflict', message: 'stale', current }, 409);
	installFetch(async ({ method, path, url, body }) => {
		const n = net.s;
		if (method === 'GET') n.gets.push(path + url.search);
		if (n.hold.has(path)) await new Promise<void>((r) => n.held.push(r));
		const parts = path.split('/'); // ['', 'api', 'v1', 'stories', key, sub]
		const key = parts[4];
		const at = n.stories.findIndex((s) => s.key === key);
		const plain = parts[3] === 'stories' && parts.length === 5 && key !== 'similar';
		if (method === 'GET') {
			if (path === '/api/v1/stories') {
				const params = [...url.searchParams.keys()];
				if (params.join() !== 'untriaged' || url.searchParams.get('untriaged') !== 'true') throw new Error(`unexpected query ${url.search}`);
				// Server order is rank, deliberately not key order (property 2).
				return n.stories.filter((s) => !s.epic && s.delivery !== 'done').sort((a, b) => b.rank - a.rank).map((s) => withReadiness(s, [{ reason: 'untriaged' }]));
			}
			if (path === '/api/v1/epics') return n.epics;
			if (path === '/api/v1/stories/similar') return n.similar[url.searchParams.get('q')!] ?? [];
			if (plain) return at >= 0 ? n.stories[at] : failure('not found', 404, 'not_found');
			throw new Error(`unexpected ${method} ${path}`);
		}
		const isComment = method === 'POST' && parts.length === 6 && parts[3] === 'stories' && parts[5] === 'comments';
		const isMove = method === 'POST' && parts.length === 6 && parts[3] === 'stories' && parts[5] === 'move';
		const isPatch = method === 'PATCH' && plain;
		if (!(isComment || isMove || isPatch)) throw new Error(`unexpected ${method} ${path}`);
		if (at < 0) return failure('not found', 404, 'not_found');
		const sent = (body ?? {}) as Sent;
		n.writes.push({ method, path, body: sent });
		const s = n.stories[at];
		if (isComment) {
			if (n.failComment) return failure('comment refused');
			const list = (n.comments[key] ??= []);
			// A keyless comment is inserted every time; a keyed one once.
			if (sent.idempotency_key == null || !list.some((c) => c.key === sent.idempotency_key)) list.push({ body: sent.body, key: sent.idempotency_key });
			return [] as Comment[];
		}
		if (sent.version !== s.version) return conflict(s.version);
		if (isMove) {
			n.stories[at] = { ...s, delivery: sent.to!, resolution: sent.resolution ?? null, version: s.version + 1 };
			return n.stories[at];
		}
		const { version: _v, ...fields } = sent;
		n.stories[at] = { ...s, ...fields, version: s.version + 1 };
		return n.stories[at];
	});
	return orig();
});

const fresh = (): Net => ({
	stories: [
		story({ key: 'GRA-12', title: 'Export shows blank page', kind: 'bug', rank: 3, description: 'Reported from /board' }),
		story({ key: 'GRA-9', title: 'Search ignores accents', kind: 'bug', rank: 1, criteria: [{ text: 'Finds café', done: true }] }),
		story({ key: 'GRA-30', title: 'Tidy the sprint page', kind: 'chore', rank: 2, delivery: 'in_progress', owner: 'ana@example.com' }),
		story({ key: 'GRA-40', title: 'Placed already', epic: 'GRA-7' })
	],
	epics: [epic({ key: 'GRA-7', title: 'Migration findings' }), epic({ key: 'GRA-608', title: 'Views' })],
	similar: {},
	comments: {},
	writes: [],
	gets: [],
	hold: new Set(),
	held: [],
	failComment: false
});

let app: ReturnType<typeof mount> | null = null;
beforeEach(() => {
	net.s = fresh();
	live.fns.length = 0;
});
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	route.fake!.reset();
	vi.useRealTimers();
});

async function open() {
	app = mount(Triage, { target: document.body });
	await vi.waitFor(() => expect(document.querySelector('[role="listbox"][aria-label="Untriaged"]')).not.toBeNull());
	flushSync();
	return {
		list: () => document.querySelector<HTMLElement>('[role="listbox"][aria-label="Untriaged"]')!,
		options: () => [...document.querySelectorAll<HTMLElement>('[role="listbox"][aria-label="Untriaged"] [role="option"]')].map((o) => o.dataset.key),
		current: () => document.querySelector<HTMLElement>('[role="option"][aria-selected="true"]')?.dataset.key,
		pane: () => document.querySelector<HTMLElement>('section.item')!,
		status: () => document.querySelector('[data-testid="triage-status"]')!.textContent!.trim()
	};
}
/** A keydown on `el` as a person types it: bubbling, cancelable. Returns it, to ask whether a handler cancelled it. */
function press(el: Element, key: string, over: KeyboardEventInit = {}): KeyboardEvent {
	const e = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...over });
	el.dispatchEvent(e);
	flushSync();
	return e;
}

describe('queue', () => {
	it('lists the untriaged items oldest first by key, asks only untriaged=true, and selects the first', async () => {
		const t = await open();
		expect(t.options()).toEqual(['GRA-9', 'GRA-12', 'GRA-30']);
		expect(t.current()).toBe('GRA-9');
		expect(t.list().getAttribute('aria-activedescendant')).toBe('triage-opt-GRA-9');
		expect(document.activeElement).toBe(t.list());
		expect(t.pane().textContent).toContain('Search ignores accents');
		expect(t.pane().querySelector('.meta a')!.getAttribute('href')).toBe('/triage?story=GRA-9'); // o, as a link
	});
	it('j and k move the current item; the arrows do too in the list', async () => {
		const t = await open();
		press(t.list(), 'j');
		expect(t.current()).toBe('GRA-12');
		press(t.list(), 'ArrowDown');
		expect(t.current()).toBe('GRA-30');
		press(t.list(), 'j'); // the last: stays
		expect(t.current()).toBe('GRA-30');
		press(t.list(), 'k');
		expect(t.current()).toBe('GRA-12');
	});
	it('j and k leave focus on the listbox', async () => {
		const t = await open();
		press(t.list(), 'j');
		expect(document.activeElement).toBe(t.list());
		press(t.list(), 'k');
		expect(document.activeElement).toBe(t.list());
	});
	it('o opens the item in the drawer without resetting scroll or focus', async () => {
		const t = await open();
		press(t.list(), 'o');
		expect(goto).toHaveBeenCalledWith('/triage?story=GRA-9', { reset: false });
	});
	it('shows delivery and owner for work in progress, with the warning', async () => {
		const t = await open();
		press(t.list(), 'j');
		press(t.list(), 'j');
		expect(t.pane().textContent).toContain('in progress, owned by ana@example.com');
		expect(t.pane().textContent).toContain('dropping it stops work');
	});
	it('s skips to the next item and says so; after the last it wraps to the first', async () => {
		const t = await open();
		press(t.list(), 's');
		expect(t.current()).toBe('GRA-12');
		await vi.waitFor(() => expect(t.status()).toBe('Skipped GRA-9.'));
		press(t.list(), 's');
		press(t.list(), 's');
		expect(t.current()).toBe('GRA-9');
		await vi.waitFor(() => expect(t.status()).toBe('Skipped GRA-30; back to the first item.'));
	});
	it('keys outside the triage area do nothing', async () => {
		const t = await open();
		press(document.body, 'j');
		expect(t.current()).toBe('GRA-9');
	});
	it('an empty queue says so, with focus on its heading', async () => {
		net.s.stories = [];
		app = mount(Triage, { target: document.body });
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('triage-empty'));
		expect(document.body.textContent).toContain('Nothing to triage');
	});
	it('Escape in a field moves focus to the item heading', async () => {
		await open();
		const field = document.querySelector<HTMLTextAreaElement>('textarea.criteria')!;
		field.focus();
		press(field, 'Escape');
		expect(document.activeElement?.id).toBe('triage-item-title');
	});
});

describe('the current item leaves the queue', () => {
	async function leave(keep: string[]) {
		net.s.stories = net.s.stories.filter((s) => keep.includes(s.key) || s.epic);
		const t = await open();
		const i = net.s.stories.findIndex((s) => s.key === t.current());
		net.s.stories[i] = { ...net.s.stories[i], epic: 'GRA-7', version: net.s.stories[i].version + 1 };
		net.s.hold.add(`/api/v1/stories/${t.current()}`); // the read of where it went is slow: the window between
		live.fns.forEach((f) => f());
		await vi.waitFor(() => expect(net.s.held.length).toBe(1));
		await settle();
		return t;
	}
	it('hides the counter and keeps the item pane while a list remains', async () => {
		await leave(['GRA-9', 'GRA-12']);
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		expect(document.querySelector('.counter')?.textContent ?? '').not.toMatch(/\d+ of \d+/);
	});
	it('the last item leaving shows no empty state and keeps focus on the item heading', async () => {
		await leave(['GRA-9']);
		expect(document.body.textContent).not.toContain('Nothing to triage'); // while the read is pending
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		await vi.waitFor(() => expect(document.querySelector('[data-testid="triage-status"]')!.textContent).not.toBe(''));
		expect(document.body.textContent).not.toContain('Nothing to triage');
		expect(document.querySelector('.counter')?.textContent ?? '').not.toMatch(/\d+ of \d+/);
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('triage-item-title'));
	});
});
