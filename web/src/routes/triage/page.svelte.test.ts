// @vitest-environment jsdom
// web/src/routes/triage/page.svelte.test.ts
// /triage: the untriaged queue oldest first, one item at a time, placed or dropped from the keyboard; drafts, Undo,
// conflicts and live changes. The fake server applies writes as the real one does (version bump, 409 on a stale version,
// placed and dropped items leave the untriaged list).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Comment, Epic, SimilarStory, Story } from '#lib/api/types.ts';
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
	/** A PATCH answers 500 (after it is recorded). */
	failPatch: boolean;
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
		if (isPatch && n.failPatch) return failure('boom');
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
	failComment: false,
	failPatch: false
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
		criteria().focus(); // and the user types in its Criteria
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.options()).toEqual(['GRA-12', 'GRA-30']));
		expect(t.current()).toBe('GRA-30');
		await settle();
		expect(document.activeElement).toBe(criteria()); // the late answer does not take the focus: the next d is a letter
	});
	it('a live reload that adds items announces them, but never over an error', async () => {
		const t = await open();
		net.s.stories.push(story({ key: 'GRA-50', title: 'Fresh report' }));
		await live.fns[0]();
		await vi.waitFor(() => expect(t.status()).toBe('1 new untriaged item.'));
		expect(t.current()).toBe('GRA-9'); // the current item does not change
		press(t.list(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toBe('Pick an epic first.'));
		net.s.stories.push(story({ key: 'GRA-51', title: 'Another report' }));
		await live.fns[0]();
		await settle();
		flushSync();
		expect(t.options()).toContain('GRA-51');
		expect(t.status()).toBe('Pick an epic first.');
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
	it('a held write disables the form of its own item only', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.hold.add('/api/v1/stories/GRA-9');
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(net.s.held).toHaveLength(1));
		expect(criteria().disabled).toBe(true);
		t.list().focus();
		press(t.list(), 'j');
		expect(t.current()).toBe('GRA-12');
		expect(criteria().disabled).toBe(false);
		net.s.hold.clear();
		net.s.held.splice(0).forEach((r) => r());
		await vi.waitFor(() => expect(t.options()).toEqual(['GRA-12', 'GRA-30']));
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
	it('a failure or a conflict is not covered by a later "new untriaged" announcement', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.failPatch = true;
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toBe('GRA-9 was not placed: boom'));
		net.s.stories.push(story({ key: 'GRA-50', title: 'Fresh report' }));
		await live.fns[0]();
		await settle();
		flushSync();
		expect(t.options()).toContain('GRA-50');
		expect(t.status()).toBe('GRA-9 was not placed: boom');
	});
	it('a conflict notice (changed meanwhile) is not covered by a later "new untriaged" announcement', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.stories[1] = { ...net.s.stories[1], title: 'Search drops accents', version: 2 };
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toContain('GRA-9 changed meanwhile'));
		const text = t.status();
		net.s.stories.push(story({ key: 'GRA-50', title: 'Fresh report' }));
		await live.fns[0]();
		await settle();
		flushSync();
		expect(t.options()).toContain('GRA-50');
		expect(t.status()).toBe(text);
	});
	it('a conflict notice (left the queue) is not covered by a later "new untriaged" announcement', async () => {
		const t = await open();
		pickEpic('GRA-7');
		net.s.stories[1] = { ...net.s.stories[1], delivery: 'done', resolution: 'dropped', version: 2 };
		press(criteria(), 'Enter', { ctrlKey: true });
		await vi.waitFor(() => expect(t.status()).toBe('GRA-9 is done (dropped).'));
		net.s.stories.push(story({ key: 'GRA-50', title: 'Fresh report' }));
		await live.fns[0]();
		await settle();
		flushSync();
		expect(t.options()).toContain('GRA-50');
		expect(t.status()).toBe('GRA-9 is done (dropped).');
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

