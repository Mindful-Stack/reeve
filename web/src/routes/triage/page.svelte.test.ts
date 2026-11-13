// @vitest-environment jsdom
// web/src/routes/triage/page.svelte.test.ts
// /triage: the untriaged queue oldest first, one item at a time, placed or dropped from the keyboard; drafts, Undo,
// conflicts and live changes. The fake server applies writes as the real one does (version bump, 409 on a stale version,
// placed and dropped items leave the untriaged list).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AddComment, Comment, EditItem, Epic, MoveStory, SimilarStory, Story } from '#lib/api/types.ts';
import { epic, story, withReadiness } from '#lib/test/fixtures.ts';
import { choose } from '#lib/test/picker.ts';
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

/** Every field any write of this page sends: a PATCH's fields, a move's, a comment's, as the schema defines them. */
type Sent = Partial<EditItem & MoveStory & AddComment>;
type Write = { method: string; path: string; body: Sent };
type Net = {
	stories: Story[];
	epics: Epic[];
	similar: Record<string, SimilarStory[]>;
	comments: Record<string, { body?: string | null; key?: string | null }[]>;
	writes: Write[];
	gets: string[];
	/** Paths whose answers wait for a call in `held`. */
	hold: Set<string>;
	held: (() => void)[];
	failComment: boolean;
	/** A PATCH answers 500 (after it is recorded). */
	failPatch: boolean;
	/** A move is recorded, then answers 500 ('fail') or a refusal no override can get past ('refuse'). */
	failMove: null | 'fail' | 'refuse';
	/** The untriaged list answers 500. */
	failList: boolean;
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
				if (n.failList) return failure('boom');
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
		if (isPatch && n.failPatch) return failure('boom');
		if (isMove && n.failMove === 'fail') return failure('boom');
		if (isMove && n.failMove === 'refuse') return json({ outcome: 'refused', reasons: [{ reason: 'untriaged' }], overridable: false });
		if (sent.version !== s.version) return conflict(s.version);
		if (isMove) {
			n.stories[at] = { ...s, delivery: sent.to!, resolution: sent.resolution ?? null, version: s.version + 1 };
			return n.stories[at];
		}
		// Place sends epic, kind, title and criteria only.
		const { version: _v, criteria, epic, kind, title } = sent;
		n.stories[at] = {
			...s,
			...(epic !== undefined && { epic }),
			...(kind && { kind }),
			...(title && { title }),
			...(criteria && { criteria }),
			version: s.version + 1
		};
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
	failComment: false,
	failPatch: false,
	failMove: null,
	failList: false
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
	it('j and k, pressed with focus on the item heading, take focus back to the listbox', async () => {
		const t = await open();
		const heading = () => document.querySelector<HTMLElement>('#triage-item-title')!;
		heading().focus();
		press(heading(), 'j');
		await vi.waitFor(() => expect(document.activeElement).toBe(t.list()));
		heading().focus();
		press(heading(), 'k');
		await vi.waitFor(() => expect(document.activeElement).toBe(t.list()));
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
	it('Previous and Next say so at the ends', async () => {
		const t = await open();
		press(t.list(), 'k');
		await vi.waitFor(() => expect(t.status()).toBe('First item.'));
		expect(t.current()).toBe('GRA-9');
		press(t.list(), 'j');
		press(t.list(), 'j');
		press(t.list(), 'j');
		await vi.waitFor(() => expect(t.status()).toBe('Last item.'));
		expect(t.current()).toBe('GRA-30');
	});
	it('shows Loading… until the first answer lands', async () => {
		net.s.hold.add('/api/v1/stories');
		app = mount(Triage, { target: document.body });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		expect(document.body.textContent).toContain('Loading…');
		expect(document.body.textContent).not.toContain('Nothing to triage');
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(document.querySelector('[role="listbox"][aria-label="Untriaged"]')).not.toBeNull());
		expect(document.body.textContent).not.toContain('Loading…');
	});
	it('a failed first load says why and offers Retry, which loads the queue', async () => {
		net.s.failList = true;
		app = mount(Triage, { target: document.body });
		await vi.waitFor(() => expect(document.querySelector('[role="alert"]')?.textContent).toContain("Couldn't load the queue: boom"));
		expect(document.body.textContent).not.toContain('Loading…');
		net.s.failList = false;
		const retry = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Retry')!;
		retry.click();
		flushSync();
		await vi.waitFor(() => expect(document.querySelector('[role="listbox"][aria-label="Untriaged"]')).not.toBeNull());
		expect(document.querySelector('[role="alert"]')).toBeNull();
		await vi.waitFor(() => expect(document.activeElement?.getAttribute('role')).toBe('listbox'));
	});
	it('a decision parameter that opens nothing leaves the keys on', async () => {
		const t = await open();
		await route.fake!.goto('/triage?decision=abc'); // the layout opens no drawer for it
		flushSync();
		press(t.list(), 'j');
		expect(t.current()).toBe('GRA-12');
		await route.fake!.goto('/triage?decision=3'); // a decision drawer: keys are off
		flushSync();
		press(t.list(), 'j');
		expect(t.current()).toBe('GRA-12');
	});
	it('key hints are hidden from assistive technology, the options carry no position attributes, and Epic is a labelled field', async () => {
		const t = await open();
		const kbds = [...document.querySelectorAll('.triage kbd')];
		expect(kbds.length).toBeGreaterThanOrEqual(4);
		expect(kbds.every((k) => k.getAttribute('aria-hidden') === 'true')).toBe(true);
		expect(document.querySelector('[role="option"][aria-posinset], [role="option"][aria-setsize]')).toBeNull();
		const label = document.querySelector('section.item label.epic')!;
		expect(label.textContent).toContain('Epic');
		expect(label.querySelector('input[role="combobox"]')).not.toBeNull();
		expect(label.querySelector('input[placeholder]')).toBeNull();
		expect(t.pane().querySelector('.keys')).toBeNull();
	});
	it('the Keys list names ?, the arrows and Cmd', async () => {
		await open();
		[...document.querySelectorAll<HTMLButtonElement>('.counter button')].find((b) => b.textContent!.startsWith('Keys'))!.click();
		flushSync();
		const keys = document.querySelector('.keys')!.textContent!;
		expect(keys).toContain('Ctrl/Cmd+Enter');
		expect(keys).toContain('↑ / ↓');
		expect(keys).toContain('?');
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
	it('hides the counter and keeps the item pane, disabled and saying where it went, while a list remains', async () => {
		const t = await leave(['GRA-9', 'GRA-12']);
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		await vi.waitFor(() => expect(t.status()).toBe('Placed elsewhere: GRA-9 is under GRA-7.'));
		expect(document.querySelector('.counter')?.textContent ?? '').not.toMatch(/\d+ of \d+/);
		expect(t.pane().textContent).toContain('Search ignores accents');
		expect(t.pane().textContent).toContain('Placed elsewhere: GRA-9 is under GRA-7.');
		expect(criteria().disabled).toBe(true);
	});
	it('the last item leaving: j then leaves the dead end for the empty state, with focus on its heading', async () => {
		const t = await leave(['GRA-9']);
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		await vi.waitFor(() => expect(t.status()).toBe('Placed elsewhere: GRA-9 is under GRA-7.'));
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('triage-item-title'));
		press(document.activeElement!, 'j');
		await vi.waitFor(() => expect(document.body.textContent).toContain('Nothing to triage'));
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('triage-empty'));
	});
	it('the last item leaving: s leaves the dead end too', async () => {
		const t = await leave(['GRA-9']);
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		await vi.waitFor(() => expect(t.status()).toBe('Placed elsewhere: GRA-9 is under GRA-7.'));
		press(document.activeElement!, 's');
		await vi.waitFor(() => expect(document.body.textContent).toContain('Nothing to triage'));
	});
	it('an item that comes back to the queue is no longer gone: its form is enabled again', async () => {
		const t = await leave(['GRA-9', 'GRA-12']);
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		await vi.waitFor(() => expect(criteria().disabled).toBe(true));
		const i = net.s.stories.findIndex((s) => s.key === 'GRA-9');
		net.s.stories[i] = { ...net.s.stories[i], epic: null, version: net.s.stories[i].version + 1 }; // someone un-placed it
		await live.fns[0]();
		await vi.waitFor(() => expect(criteria().disabled).toBe(false));
		expect(t.pane().textContent).not.toContain('Placed elsewhere');
		expect(t.current()).toBe('GRA-9');
		expect(document.querySelector('.counter')?.textContent).toMatch(/1 of 2/);
	});
	it('a done item that left is not told that dropping it stops work, nor is one placed elsewhere in progress', async () => {
		net.s.stories = net.s.stories.filter((s) => s.key === 'GRA-30' || s.epic);
		const t = await open();
		expect(t.current()).toBe('GRA-30');
		expect(t.pane().querySelector('.warn')).not.toBeNull(); // in progress and still here
		net.s.hold.add('/api/v1/stories/GRA-30');
		const i = net.s.stories.findIndex((s) => s.key === 'GRA-30');
		net.s.stories[i] = { ...net.s.stories[i], delivery: 'done', resolution: 'dropped', version: 2 };
		live.fns.forEach((f) => f());
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		await vi.waitFor(() => expect(t.status()).toBe('GRA-30 is done (dropped).'));
		expect(t.pane().querySelector('.warn')).toBeNull();
	});
	it('a read that finds the item untriaged again (a race) selects the queue\'s first item instead of a dead pane', async () => {
		const t = await open();
		net.s.stories[1] = { ...net.s.stories[1], epic: 'GRA-7', version: 2 };
		net.s.hold.add('/api/v1/stories/GRA-9');
		live.fns.forEach((f) => f());
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		net.s.stories[1] = { ...net.s.stories[1], epic: null, version: 3 }; // un-placed before the read lands
		net.s.hold.clear();
		net.s.held.forEach((r) => r());
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(t.status()).not.toContain('changed');
		expect(document.querySelector('section.item')).not.toBeNull();
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

/** Picks `epic` in the current item's Picker as a person would (types, then clicks the option). */
function pickEpic(epicKey: string) {
	choose(document.querySelector('section.item')!, 'Epic', epicKey);
	flushSync();
}
const criteria = () => document.querySelector<HTMLTextAreaElement>('textarea.criteria')!;
function type(el: HTMLInputElement | HTMLTextAreaElement, text: string) {
	el.value = text;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
}

describe('place', () => {
	it('Ctrl+Enter places with only the changed fields and the version, links the item, and the next item is current and focused', async () => {
		const t = await open();
		pickEpic('GRA-7');
		expect(document.activeElement).toBe(criteria()); // picking moves on to Criteria
		type(criteria(), 'Finds café\nFinds naïve');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.writes).toEqual([
			{
				method: 'PATCH',
				path: '/api/v1/stories/GRA-9',
				body: { version: 1, epic: 'GRA-7', criteria: [{ text: 'Finds café', done: true }, { text: 'Finds naïve', done: false }] }
			}
		]);
		expect(t.options()).toEqual(['GRA-12', 'GRA-30']);
		await vi.waitFor(() => expect(t.status()).toBe('Placed GRA-9 under GRA-7.'));
		expect(document.querySelector('.notice a')!.getAttribute('href')).toBe('/triage?story=GRA-9');
		expect(document.activeElement).toBe(t.list());
	});
	it('without an epic, Place focuses the Picker and says so', async () => {
		const t = await open();
		press(t.list(), 'Enter', { ctrlKey: true });
		await settle(); // a write would be recorded only after several awaits: let them all run before asserting none
		expect(net.s.writes).toEqual([]);
		expect(document.activeElement?.getAttribute('role')).toBe('combobox');
		await vi.waitFor(() => expect(t.status()).toBe('Pick an epic first.'));
	});
	it('t cycles the kind and says so: the select shows it and Place sends it', async () => {
		const t = await open();
		press(t.list(), 't'); // GRA-9 is a bug: bug -> chore
		expect(document.querySelector<HTMLSelectElement>('section.item select')!.value).toBe('chore');
		await vi.waitFor(() => expect(t.status()).toBe('Kind: chore.'));
		pickEpic('GRA-7');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.writes).toHaveLength(1));
		expect(net.s.writes[0].body).toEqual({ version: 1, epic: 'GRA-7', kind: 'chore' });
	});
	it('a draft survives moving away and back', async () => {
		const t = await open();
		type(criteria(), 'Typed here');
		press(t.list(), 'j');
		press(t.list(), 'k');
		expect(criteria().value).toBe('Typed here');
	});
	it('a 409 keeps the typed fields, takes the server\'s others, names the field both changed, and gives focus back', async () => {
		const t = await open();
		pickEpic('GRA-7');
		const title = document.querySelector<HTMLInputElement>('section.item input[name="title"]')!;
		type(title, 'Search ignores accents (é)');
		// A colleague changes the title and kind meanwhile, with no live refresh yet: the server is ahead of the page.
		net.s.stories[1] = { ...net.s.stories[1], title: 'Search drops accents', kind: 'story', version: 2 };
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toContain('GRA-9 changed meanwhile'));
		expect(t.status()).toContain('Title is now "Search drops accents"');
		expect(title.value).toBe('Search ignores accents (é)');
		expect(document.querySelector<HTMLSelectElement>('section.item select')!.value).toBe('story');
		await vi.waitFor(() => expect(document.activeElement).toBe(t.list())); // the keys work again
		press(criteria(), 'Enter', { ctrlKey: true }); // placing again sends against the reloaded version
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.writes.at(-1)!.body.version).toBe(2);
	});
	it('the current item placed elsewhere (live) says where it went, disables the form and gives focus back; j moves on', async () => {
		const t = await open();
		criteria().focus();
		net.s.stories[1] = { ...net.s.stories[1], epic: 'GRA-608', version: 2 };
		await live.fns[0]();
		await vi.waitFor(() => expect(t.status()).toBe('Placed elsewhere: GRA-9 is under GRA-608.'));
		expect(t.pane().textContent).toContain('Placed elsewhere: GRA-9 is under GRA-608.');
		expect(criteria().disabled).toBe(true);
		// A browser drops focus from the disabled field to the page, where no key works: the page takes it back to the list.
		await vi.waitFor(() => expect(document.activeElement).toBe(t.list()));
		press(t.list(), 'j');
		expect(t.current()).toBe('GRA-12');
	});
	it('a 409 because the item left the queue says where it went and disables the form, as a live change does', async () => {
		const t = await open();
		pickEpic('GRA-7');
		// A colleague dropped it meanwhile, with no live refresh yet.
		net.s.stories[1] = { ...net.s.stories[1], delivery: 'done', resolution: 'dropped', version: 2 };
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toBe('GRA-9 is done (dropped).'));
		expect(criteria().disabled).toBe(true);
		expect(t.options()).toEqual(['GRA-12', 'GRA-30']);
		expect(net.s.writes.filter((w) => w.method === 'PATCH')).toHaveLength(1);
	});
	it('a late Place answer, with the user on another item, does not move the selection', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1)); // the PATCH has reached the server and waits
		t.list().focus();
		press(t.list(), 'j');
		press(t.list(), 'j'); // now on GRA-30
		await settle(); // j's own focus move has landed
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.options()).toEqual(['GRA-12', 'GRA-30']));
		expect(t.current()).toBe('GRA-30');
	});
	it('a live reload that adds items adds them to the queue and says nothing', async () => {
		const t = await open();
		net.s.stories.push(story({ key: 'GRA-50', title: 'Fresh report' }));
		await live.fns[0]();
		await vi.waitFor(() => expect(t.options()).toContain('GRA-50'));
		expect(t.status()).toBe('');
		expect(t.current()).toBe('GRA-9'); // the current item does not change
	});
	it('t does nothing while a write for the current item is in flight', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		press(t.list(), 't');
		expect(document.querySelector<HTMLSelectElement>('section.item select')!.value).toBe('bug');
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.writes).toEqual([{ method: 'PATCH', path: '/api/v1/stories/GRA-9', body: { version: 1, epic: 'GRA-7' } }]);
	});
	it('a held write disables Place, Drop and the fields of every item, not only its own', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		expect(criteria().disabled).toBe(true);
		t.list().focus();
		press(t.list(), 'j');
		expect(t.current()).toBe('GRA-12');
		expect(criteria().disabled).toBe(true);
		expect([...t.pane().querySelectorAll<HTMLButtonElement>('form button')].filter((b) => !b.textContent!.startsWith('Skip')).every((b) => b.disabled)).toBe(true);
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.options()).toEqual(['GRA-12', 'GRA-30']));
		expect(criteria().disabled).toBe(false);
	});
	it('a reload while the current item is being placed does not read where it went', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		net.s.stories[1] = { ...net.s.stories[1], epic: 'GRA-7' }; // the same write, seen by a reload before its answer
		await live.fns[0]();
		await settle();
		expect(net.s.gets).not.toContain('/api/v1/stories/GRA-9');
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.status()).toBe('Placed GRA-9 under GRA-7.'));
	});
	it('a late 409 for an item the user left does not mark the item the user is on as gone, nor move the selection', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		t.list().focus();
		press(t.list(), 'j');
		press(t.list(), 'j'); // now on GRA-30
		net.s.stories[1] = { ...net.s.stories[1], epic: 'GRA-608', version: 2 }; // a colleague placed GRA-9 meanwhile
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.status()).toBe('Placed elsewhere: GRA-9 is under GRA-608.'));
		await settle();
		expect(t.options()).toEqual(['GRA-12', 'GRA-30']);
		expect(t.current()).toBe('GRA-30');
		expect(t.pane().textContent).toContain('Tidy the sprint page');
		expect(t.pane().textContent).not.toContain('Placed elsewhere');
		expect(criteria().disabled).toBe(false);
	});
	it('a failed write says which item was not placed and why', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.failPatch = true;
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toBe('GRA-9 was not placed: boom'));
		expect(t.current()).toBe('GRA-9');
	});
	it('a late answer does not take focus from a control the user moved to inside the triage area', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		const keys = [...document.querySelectorAll<HTMLButtonElement>('.counter button')].find((b) => b.textContent!.startsWith('Keys'))!;
		keys.focus();
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		await settle();
		expect(document.activeElement).toBe(keys);
	});
	it('a late answer does not take focus from a control the user moved to outside the triage area', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		const outside = document.createElement('button');
		document.body.append(outside);
		outside.focus();
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		await settle();
		expect(document.activeElement).toBe(outside);
	});
	it('a held write blocks a second Place', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		press(criteria(), 'Enter', { ctrlKey: true });
		await settle();
		expect(net.s.held).toHaveLength(1); // the second Place sent nothing
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.writes.filter((w) => w.method === 'PATCH')).toHaveLength(1);
	});
});

describe('drop', () => {
	it('d then a separate d drops with the reviewed version; a held d (repeat) never does', async () => {
		const t = await open();
		press(t.list(), 'd');
		press(t.list(), 'd', { repeat: true });
		await vi.waitFor(() => expect(t.status()).toBe('Drop GRA-9? Press d or click again.'));
		expect(net.s.writes).toEqual([]);
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.writes).toEqual([{ method: 'POST', path: '/api/v1/stories/GRA-9/move', body: { version: 1, to: 'done', resolution: 'dropped' } }]);
		await vi.waitFor(() => expect(t.status()).toBe('Dropped GRA-9.'));
	});
	it('Escape ends the arm and says so; the next d arms again', async () => {
		const t = await open();
		press(t.list(), 'd');
		expect(dropButton().textContent).toContain('Confirm drop');
		press(t.list(), 'Escape');
		expect(t.status()).toBe('Drop cancelled.');
		expect(dropButton().textContent).not.toContain('Confirm drop');
		press(t.list(), 'd'); // arms again, does not drop
		await settle();
		expect(net.s.writes).toEqual([]);
	});
	it('no clock ends the arm: a minute later the button is still armed and the second d drops', async () => {
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
		const t = await open();
		press(t.list(), 'd');
		await settle();
		vi.advanceTimersByTime(60_000);
		await settle();
		flushSync();
		expect(t.status()).toBe('Drop GRA-9? Press d or click again.');
		expect(dropButton().textContent).toContain('Confirm drop');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(net.s.writes).toHaveLength(1));
	});
	it('a live change to the armed item makes the second d arm again instead of dropping', async () => {
		const t = await open();
		press(t.list(), 'd');
		net.s.stories[1] = { ...net.s.stories[1], title: 'Search ignores accents!', version: 2 };
		await live.fns[0]();
		await vi.waitFor(() => expect(t.pane().textContent).toContain('Search ignores accents!'));
		press(t.list(), 'd');
		await settle();
		expect(net.s.writes).toEqual([]);
		press(t.list(), 'd'); // the arm now names version 2
		await vi.waitFor(() => expect(net.s.writes).toEqual([{ method: 'POST', path: '/api/v1/stories/GRA-9/move', body: { version: 2, to: 'done', resolution: 'dropped' } }]));
	});
	it('the Drop button works from the keyboard: Enter twice; work in progress says it stops work', async () => {
		const t = await open();
		press(t.list(), 'j');
		press(t.list(), 'j'); // GRA-30, in progress
		const button = [...t.pane().querySelectorAll('button')].find((b) => b.textContent!.startsWith('Drop'))!;
		const enter = () => {
			button.focus();
			const e = press(button, 'Enter'); // the keydown reaches the area first, as in a browser
			expect(e.defaultPrevented).toBe(false); // so the browser goes on to activate the button:
			button.click();
			flushSync();
		};
		enter();
		expect(button.textContent).toContain('Confirm drop');
		expect(t.status()).toBe('Drop GRA-30 and stop active work (Undo returns it as todo)? Press d or click again.');
		expect(net.s.writes).toEqual([]);
		enter();
		await vi.waitFor(() => expect(net.s.writes).toHaveLength(1));
	});
	it('u undoes a drop of work in progress: back to todo, and current again', async () => {
		const t = await open();
		press(t.list(), 'j');
		press(t.list(), 'j'); // GRA-30, in progress
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.options()).toEqual(['GRA-9', 'GRA-12']));
		press(t.list(), 'u');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-30'));
		expect(net.s.writes.at(-1)).toEqual({ method: 'POST', path: '/api/v1/stories/GRA-30/move', body: { version: 2, to: 'todo' } });
		expect(net.s.stories[2].delivery).toBe('todo');
		await vi.waitFor(() => expect(t.status()).toBe('Undone: GRA-30 is back in todo.'));
	});
	it('u undoes a place, also after a Skip: every sent field back, the draft restored, announced once', async () => {
		const t = await open();
		pickEpic('GRA-7');
		type(criteria(), 'Finds café\nFinds naïve');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		press(t.list(), 's'); // another message: the Undo stays, and still names its item
		await vi.waitFor(() => expect(t.status()).toBe('Skipped GRA-12.'));
		expect(document.querySelector('.notice button')!.textContent).toContain('Undo place GRA-9');
		press(t.list(), 'u');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-9'));
		expect(net.s.writes.at(-1)!.body).toEqual({ version: 2, epic: null, criteria: [{ text: 'Finds café', done: true }] });
		expect(criteria().value).toBe('Finds café\nFinds naïve');
		await vi.waitFor(() => expect(t.status()).toBe('Undone: GRA-9 is back in the queue.'));
		await live.fns[0](); // the write's own live refetch: the item is back already, not new
		await settle();
		expect(t.status()).toBe('Undone: GRA-9 is back in the queue.');
	});
	it('undo after someone changed the item writes nothing and says so', async () => {
		const t = await open();
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		net.s.stories[1] = { ...net.s.stories[1], version: 9 };
		const before = net.s.writes.length;
		press(t.list(), 'u');
		await vi.waitFor(() => expect(t.status()).toBe('GRA-9 changed since; undo it in the drawer.'));
		expect(net.s.writes.length).toBe(before + 1); // the one refused move, nothing after it
	});
	it('a reload that was in flight when Undo landed does not take the restored item out again', async () => {
		const t = await open();
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		net.s.hold.add('/api/v1/stories');
		const reloading = live.fns[0](); // its list read waits
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		press(t.list(), 'u');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-9'));
		net.s.stories[1] = { ...net.s.stories[1], epic: 'GRA-7' }; // the stale read now answers without GRA-9
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await reloading;
		await settle();
		flushSync();
		expect(t.options()).toContain('GRA-9');
	});
});

const dropButton = () => [...document.querySelectorAll<HTMLButtonElement>('section.item button')].find((b) => /^(Drop|Confirm drop)/.test(b.textContent!))!;

describe('drop and focus, review fixes', () => {
	it('a held Enter on the Drop button is cancelled, so it does not click again and confirm', async () => {
		await open();
		const button = dropButton();
		button.focus();
		expect(press(button, 'Enter', { repeat: true }).defaultPrevented).toBe(true);
		expect(press(button, 'Enter').defaultPrevented).toBe(false);
	});
	it('dropping the last item with focus on the item heading leaves focus where u works', async () => {
		net.s.stories = [net.s.stories[1]];
		const t = await open();
		const heading = document.querySelector<HTMLElement>('#triage-item-title')!;
		heading.focus();
		press(heading, 'd');
		press(heading, 'd');
		await vi.waitFor(() => expect(document.body.textContent).toContain('Nothing to triage'));
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('triage-empty'));
		press(document.activeElement!, 'u');
		await vi.waitFor(() => expect(t.status()).toBe('Undone: GRA-9 is back in todo.'));
	});
	it('a late answer does not take focus from an enabled button of the item (Skip)', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		const skip = [...document.querySelectorAll<HTMLButtonElement>('section.item button')].find((b) => b.textContent!.startsWith('Skip'))!;
		skip.focus();
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		await settle();
		expect(document.activeElement).toBe(skip);
	});
	it('the drop sends the version the draft was reviewed at, not the live story\'s', async () => {
		const t = await open();
		const title = document.querySelector<HTMLInputElement>('section.item input[name="title"]')!;
		type(title, 'Search ignores accents (é)');
		net.s.stories[1] = { ...net.s.stories[1], title: 'Search drops accents', version: 2 }; // both changed the title: the draft is kept at version 1
		await live.fns[0]();
		await vi.waitFor(() => expect(t.pane().textContent).toContain('Search drops accents'));
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toContain('GRA-9 changed meanwhile'));
		expect(net.s.writes).toEqual([{ method: 'POST', path: '/api/v1/stories/GRA-9/move', body: { version: 1, to: 'done', resolution: 'dropped' } }]);
	});
	it('a held drop blocks further d presses', async () => {
		const t = await open();
		net.s.hold.add('/api/v1/stories/GRA-9/move');
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		press(t.list(), 'd');
		press(t.list(), 'd');
		await settle();
		expect(net.s.held).toHaveLength(1);
		expect(dropButton().textContent).not.toContain('Confirm drop');
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.writes).toHaveLength(1);
	});
	it('Undo is one use: the button goes, and another u writes nothing; and it waits for a write in flight', async () => {
		const t = await open();
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		press(t.list(), 'u');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-9'));
		await settle();
		expect(document.querySelector('.notice button')).toBeNull();
		const before = net.s.writes.length;
		press(t.list(), 'u');
		await settle();
		expect(net.s.writes).toHaveLength(before);
	});
	it('u does nothing while another write is in flight', async () => {
		const t = await open();
		pickEpic('GRA-7');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-12');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		const before = net.s.writes.length;
		press(t.list(), 'u'); // GRA-9's Undo is ready, but GRA-12's write is in flight
		await settle();
		expect(net.s.writes).toHaveLength(before);
		expect(document.querySelector('.notice button')!.textContent).toContain('Undo place GRA-9');
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.options()).toEqual(['GRA-30']));
	});
	it('the undone place comes back with its fields but no epic: placing again asks for one', async () => {
		const t = await open();
		pickEpic('GRA-7');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		press(t.list(), 'u');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-9'));
		await vi.waitFor(() => expect(t.status()).toBe('Undone: GRA-9 is back in the queue.'));
		const before = net.s.writes.length;
		press(t.list(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toBe('Pick an epic first.'));
		expect(net.s.writes).toHaveLength(before);
	});
	it('a refused Undo of a drop keeps the server\'s reasons', async () => {
		const t = await open();
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		net.s.failMove = 'refuse';
		press(t.list(), 'u');
		await vi.waitFor(() => expect(t.status()).toMatch(/^Undo failed: GRA-9 was not moved to todo: /));
		expect(t.status()).toContain('untriaged');
	});
});

const hit = (key: string, title: string, over: Partial<SimilarStory> = {}): SimilarStory => ({ key, title, kind: 'bug', delivery: 'todo', ...over });
const similarGets = () => net.s.gets.filter((g) => g.startsWith('/api/v1/stories/similar'));
const hits = () => [...document.querySelectorAll('section.item section.similar li')];

describe('similar and duplicates', () => {
	beforeEach(() => {
		net.s.similar['Search ignores accents'] = [
			hit('GRA-9', 'Search ignores accents'), // the item itself: the server does not leave it out
			hit('GRA-3', 'Accents break search', { delivery: 'done', resolution: 'dropped' }),
			hit('GRA-4', 'Search and diacritics')
		];
		net.s.similar['Export shows blank page'] = [hit('GRA-5', 'Export is blank')];
	});
	it('lists the hits without the item itself, numbered, with the resolution of a done one; asked once, not on a live reload', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		const panel = t.pane().querySelector('section.similar')!;
		expect(panel.textContent).not.toContain('GRA-9');
		expect([...panel.querySelectorAll('.n')].map((n) => n.textContent)).toEqual(['1', '2']);
		expect(panel.textContent).toContain('done, dropped');
		// Parsed, not compared as text: openapi-fetch's encoding of the space is its own business.
		const u = new URL(similarGets()[0], 'http://localhost');
		expect([u.searchParams.get('q'), u.searchParams.get('limit')]).toEqual(['Search ignores accents', '6']);
		await live.fns[0]();
		await settle();
		expect(similarGets()).toHaveLength(1);
	});
	it('2 marks the second hit and says so; d d comments with an idempotency key, then drops', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		press(t.list(), '2');
		await vi.waitFor(() => expect(t.status()).toBe('Marked as a duplicate of GRA-4.'));
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toBe('Drop GRA-9 as a duplicate of GRA-4? Press d or click again.'));
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.writes.map((w) => [w.method, w.path, w.body])).toEqual([
			['POST', '/api/v1/stories/GRA-9/comments', { body: 'Duplicate of GRA-4.', idempotency_key: 'triage-dup:GRA-9:GRA-4' }],
			['POST', '/api/v1/stories/GRA-9/move', { version: 1, to: 'done', resolution: 'dropped' }]
		]);
	});
	it('marking a duplicate ends an armed drop: the next d arms again, naming the duplicate', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		press(t.list(), 'd');
		press(t.list(), '2');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toBe('Drop GRA-9 as a duplicate of GRA-4? Press d or click again.'));
		expect(net.s.writes).toEqual([]);
	});
	it('the same digit again unmarks', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		press(t.list(), '1');
		press(t.list(), '1');
		await vi.waitFor(() => expect(t.status()).toBe('Duplicate mark cleared.'));
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toBe('Drop GRA-9? Press d or click again.'));
	});
	it('a failed comment moves nothing; a 409 after the comment landed says so; the retry posts no second comment', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		press(t.list(), '1');
		net.s.failComment = true;
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toContain('GRA-9 was not dropped'));
		expect(net.s.writes.some((w) => w.path.endsWith('/move'))).toBe(false);
		net.s.failComment = false;
		net.s.stories[1] = { ...net.s.stories[1], version: 2 }; // a colleague's edit, not yet seen live
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toMatch(/^GRA-9 changed meanwhile.*The duplicate comment is saved\.$/));
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.comments['GRA-9']).toEqual([{ body: 'Duplicate of GRA-3.', key: 'triage-dup:GRA-9:GRA-3' }]); // posted once
	});
	it('Inspect links the hit into the drawer over this page; with the drawer open the item and its draft stay', async () => {
		const t = await open();
		type(criteria(), 'Kept');
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		const link = [...t.pane().querySelectorAll<HTMLAnchorElement>('section.similar a')].find((a) => a.textContent === 'Inspect GRA-4')!;
		expect(link.getAttribute('href')).toBe('/triage?story=GRA-4'); // jsdom has no router: the link, not a navigation
		await route.fake!.goto(link.getAttribute('href')!); // what SvelteKit does on the click
		flushSync();
		press(t.list(), 'j'); // keys are off while the drawer is open
		expect(t.current()).toBe('GRA-9');
		expect(criteria().value).toBe('Kept');
	});
	it('a short title asks nothing and shows nothing similar', async () => {
		net.s.stories[1] = { ...net.s.stories[1], title: 'Accents' };
		const t = await open();
		expect(t.pane().textContent).toContain('Nothing similar.');
		expect(similarGets()).toEqual([]);
	});
	it('a similar answer for an item the user has left is dropped, whichever answers first', async () => {
		net.s.hold.add('/api/v1/stories/similar');
		const t = await open();
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1)); // GRA-9's request waits
		await settle();
		expect(net.s.held).toHaveLength(1); // one request on load, not two
		press(t.list(), 'j');
		await vi.waitFor(() => expect(net.s.held).toHaveLength(2)); // and GRA-12's
		net.s.held[1](); // the newest answers first
		await vi.waitFor(() => expect(t.pane().textContent).toContain('Export is blank'));
		net.s.held[0](); // then the oldest
		await settle();
		flushSync();
		expect(t.pane().textContent).toContain('Export is blank');
		expect(t.pane().textContent).not.toContain('Accents break search');
	});
});

describe('triage review fixes', () => {
	beforeEach(() => {
		net.s.similar['Search ignores accents'] = [
			hit('GRA-9', 'Search ignores accents'),
			hit('GRA-3', 'Accents break search', { delivery: 'done', resolution: 'dropped' }),
			hit('GRA-4', 'Search and diacritics', { resolution: 'dropped' }) // todo with a stale resolution: not shown
		];
		net.s.similar['Export shows blank page'] = [hit('GRA-5', 'Export is blank')];
	});
	/** A Place for GRA-9 whose PATCH answer is held; returns what releases it. */
	async function heldPlace() {
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		return async (t: Awaited<ReturnType<typeof open>>) => {
			net.s.hold.clear();
			net.s.held.splice(0).forEach((r) => r());
			await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
			await settle();
		};
	}
	it('a late answer does not take focus from a non-control outside the triage area (the drawer heading)', async () => {
		const t = await open();
		const release = await heldPlace();
		const heading = document.createElement('h2');
		heading.tabIndex = -1;
		document.body.append(heading);
		heading.focus();
		await release(t);
		expect(document.activeElement).toBe(heading);
	});
	it.each([
		['a link', () => Object.assign(document.createElement('a'), { href: '/somewhere', textContent: 'link' })],
		['an input', () => document.createElement('input')],
		['a select', () => document.createElement('select')]
	])('a late answer does not take focus from %s inside the triage area', async (_name, make) => {
		const t = await open();
		const release = await heldPlace();
		const el = make();
		document.querySelector('.triage')!.append(el);
		el.focus();
		expect(document.activeElement).toBe(el);
		await release(t);
		expect(document.activeElement).toBe(el);
	});
	it('marking during a held duplicate drop does nothing; the retry posts no second comment', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		press(t.list(), '1');
		net.s.stories[1] = { ...net.s.stories[1], version: 2 }; // the move will 409 after the comment
		net.s.hold.add('/api/v1/stories/GRA-9/comments');
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		press(t.list(), '2'); // a write is in flight: ignored
		expect(t.status()).not.toContain('Marked');
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.status()).toMatch(/^GRA-9 changed meanwhile.*The duplicate comment is saved\.$/));
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.current()).toBe('GRA-12'));
		expect(net.s.comments['GRA-9']).toEqual([{ body: 'Duplicate of GRA-3.', key: 'triage-dup:GRA-9:GRA-3' }]);
	});
	it('selecting the current item again asks for its similar list no second time', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		document.querySelector<HTMLElement>('#triage-opt-GRA-9')!.click();
		flushSync();
		await settle();
		expect(similarGets()).toHaveLength(1);
		expect(hits()).toHaveLength(2);
	});
	it('a digit marks nothing while the similar list is not the current item\'s, and its hits are not shown', async () => {
		net.s.hold.add('/api/v1/stories/similar');
		const t = await open();
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		net.s.held[0]();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		press(t.list(), 'j'); // GRA-12: its own list is on the way, GRA-9's is still the stored one
		await vi.waitFor(() => expect(net.s.held).toHaveLength(2));
		expect(t.current()).toBe('GRA-12');
		expect(t.pane().textContent).not.toContain('Accents break search');
		press(t.list(), '1');
		expect(t.status()).not.toContain('Marked');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toBe('Drop GRA-12? Press d or click again.'));
	});
	it('the similar list is read again after a conflict that changed the title', async () => {
		net.s.similar['Accents break search'] = [hit('GRA-6', 'Diacritics in search')];
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		net.s.stories[1] = { ...net.s.stories[1], title: 'Accents break search', version: 2 };
		pickEpic('GRA-7');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toContain('changed meanwhile'));
		await vi.waitFor(() => expect(t.pane().textContent).toContain('Diacritics in search'));
		expect(similarGets()).toHaveLength(2);
	});
	it('a failure after a duplicate comment says what failed first, then that the comment is saved: a non-409 failure and a refusal of the move after a duplicate comment', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		press(t.list(), '1');
		net.s.failMove = 'fail';
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toBe('GRA-9 was not dropped (the duplicate comment is saved): boom'));
		net.s.failMove = 'refuse';
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toMatch(/^GRA-9 was not dropped \(the duplicate comment is saved\): GRA-9 was not moved to done/));
	});
	it('without a duplicate the same failures say "was not dropped"', async () => {
		const t = await open();
		net.s.failMove = 'fail';
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toBe('GRA-9 was not dropped: boom'));
		net.s.failMove = 'refuse';
		press(t.list(), 'd');
		press(t.list(), 'd');
		await vi.waitFor(() => expect(t.status()).toMatch(/^GRA-9 was not dropped: GRA-9 was not moved to done/));
	});
	it('Inspect keeps scroll and focus (reset=false); only a done hit shows its resolution', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		for (const a of t.pane().querySelectorAll('section.similar a')) expect(a.getAttribute('data-sveltekit-reset')).toBe('false');
		const [done, todo] = hits();
		expect(done.textContent).toContain('done, dropped');
		expect(todo.textContent).not.toContain('dropped');
	});
	it('the Duplicate button marks, shows it pressed, and a second click clears it', async () => {
		const t = await open();
		await vi.waitFor(() => expect(hits()).toHaveLength(2));
		const button = (n: number) => [...t.pane().querySelectorAll<HTMLButtonElement>('section.similar button')].find((b) => b.textContent!.trim() === `Duplicate of this (${n})`)!;
		expect(button(2).getAttribute('aria-pressed')).toBe('false');
		button(2).click();
		flushSync();
		await vi.waitFor(() => expect(t.status()).toBe('Marked as a duplicate of GRA-4.'));
		expect(button(2).getAttribute('aria-pressed')).toBe('true');
		expect(button(1).getAttribute('aria-pressed')).toBe('false');
		expect(hits()[1].classList.contains('marked')).toBe(true);
		button(2).click();
		flushSync();
		await vi.waitFor(() => expect(t.status()).toBe('Duplicate mark cleared.'));
		expect(button(2).getAttribute('aria-pressed')).toBe('false');
	});
});
