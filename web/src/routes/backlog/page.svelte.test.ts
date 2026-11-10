// @vitest-environment jsdom
// The backlog: the ranked todo list and the only epic index the app has. Every epic is listed, including one
// with no initiative and no waiting stories; an initiative with no epics says so; a story whose epic is not loaded is
// still shown; Move up ranks through the shared rank write and announces the new position.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { goto } from '$app/navigation';
import { page } from '$app/state';
import type { Epic, Initiative, Story, StoryWithReadiness } from '#lib/api/types.ts';
import { epic, initiative, withReadiness } from '#lib/test/fixtures.ts';

/** `untriaged` puts the server's untriaged reason on the row (the page never derives it from kind and epic). */
const row = (key: string, rank: number, epic: string | null, ready = true, kind: Story['kind'] = 'story', untriaged = false, over: Partial<Story> = {}) =>
	withReadiness(
		{ key, title: `Title ${key}`, kind, epic, rank, ...over },
		untriaged ? [{ reason: 'untriaged' }] : ready ? [] : [{ reason: 'no_criteria' }]
	);

const data = vi.hoisted(() => {
	const loose: Epic = { key: 'E-1', title: 'Loose epic', initiative: null, brief: '', version: 1, discovery_complete: null, external_ref: null, urgent_mark: null, urgent_sources: [] };
	const fresh = () => ({
		/** Every story the fake server holds, in any delivery state; a move changes the state, as the server does. */
		stories: [] as StoryWithReadiness[],
		epics: [{ ...loose }],
		initiatives: [] as Initiative[],
		posts: [] as { path: string; key: string; body: unknown }[],
		patches: [] as { key: string; body: { version: number; epic?: string | null } }[],
		storyGets: 0,
		/** The query of every stories GET, in the order asked. */
		queries: [] as Record<string, unknown>[],
		failPatch: false,
		failGets: false,
		/** While set, a stories GET whose query is not the plain `delivery=todo` fails (the full list still answers). */
		failFiltered: false,
		refuseMoves: true,
		/** While set, each stories GET answers (with the list as it was when asked) only when its `held` entry is called. */
		hold: false,
		held: [] as (() => void)[],
		/** While set, each create POST answers only when its `pending` entry is called. */
		deferCreates: false,
		pending: [] as (() => void)[],
		liveFns: [] as (() => unknown)[]
	});
	return { ...fresh(), fresh };
});
/** The fake server's story list: the plain `delivery=todo` query is the todo list in rank order; any other query is
 *  filtered here by the params the tests use, in the server's order (delivery state, then rank; under `sort=relevance`
 *  title matches first). A param it does not implement throws, so no test passes on a filter that was never applied. */
const ORDER = ['todo', 'in_progress', 'in_review', 'testing', 'done'];
const IMPLEMENTED = new Set(['delivery', 'done', 'epic', 'initiative', 'kind', 'label', 'ready', 'q', 'sort']);
function answer(query: Record<string, unknown>): StoryWithReadiness[] {
	for (const name of Object.keys(query)) if (!IMPLEMENTED.has(name)) throw new Error(`the fake server does not implement ${name}`);
	const q = (query.q as string | undefined)?.toLowerCase();
	const tier = (s: Story) => (query.sort === 'relevance' && q && s.title.toLowerCase().includes(q) ? 0 : 1);
	return data.stories
		.filter(({ story: s, readiness }) => {
			if (query.delivery && s.delivery !== query.delivery) return false;
			if (query.done === 'none' && s.delivery === 'done') return false;
			if (query.epic && s.epic !== query.epic) return false;
			if (query.kind && s.kind !== query.kind) return false;
			if (query.initiative && data.epics.find((e) => e.key === s.epic)?.initiative !== query.initiative) return false;
			if (query.label && !s.labels.includes(query.label as string)) return false;
			if (query.ready && !readiness.ready) return false;
			if (query.q && !`${s.title} ${s.description}`.toLowerCase().includes((query.q as string).toLowerCase())) return false;
			return true;
		})
		.sort(
			(a, b) =>
				tier(a.story) - tier(b.story) || ORDER.indexOf(a.story.delivery) - ORDER.indexOf(b.story.delivery) || a.story.rank - b.story.rank
		);
}
const plain = (query: Record<string, unknown>) => query.delivery === 'todo' && Object.keys(query).length === 1;
vi.mock('#lib/api/client.ts', async (orig) => {
	const fixtures = await import('#lib/test/fixtures.ts');
	const ok = (value: unknown) => Promise.resolve({ data: { outcome: 'ok', value }, response: { status: 200 } as Response });
	const fail = (message: string) => Promise.resolve({ error: { outcome: 'error', code: 'internal', message }, response: { status: 500 } as Response });
	return {
		...(await orig<typeof import('#lib/api/client.ts')>()),
		api: {
			GET: (path: string, init?: { params?: { query?: Record<string, unknown> } }) => {
				if (data.failGets) return fail('boom');
				if (!path.endsWith('/stories')) return ok(path.endsWith('/epics') ? data.epics : data.initiatives);
				const query = init?.params?.query ?? {};
				data.queries.push(query);
				data.storyGets++;
				if (data.failFiltered && !plain(query)) return fail('filter failed');
				const asked = answer(query);
				if (data.hold) return new Promise((resolve) => data.held.push(() => resolve(ok(asked))));
				return ok(asked);
			},
			PATCH: (_path: string, init: { params: { path: { key: string } }; body: { version: number; epic?: string | null } }) => {
				const key = init.params.path.key;
				if (data.failPatch) return fail('patch failed');
				data.patches.push({ key, body: init.body });
				data.stories = data.stories.map((s) =>
					s.story.key === key ? { ...s, story: { ...s.story, epic: init.body.epic ?? null, version: 2 } } : s
				);
				return ok(null);
			},
			POST: (path: string, init: { params?: { path: { key: string } }; body: { before?: string; title?: string } }) => {
				if (path === '/api/v1/stories' || path === '/api/v1/epics') {
					data.posts.push({ path, key: '', body: init.body });
					const made = ok(
						path === '/api/v1/epics'
							? fixtures.epic({ key: 'NEW-1', title: init.body.title })
							: fixtures.story({ key: 'NEW-1', title: init.body.title })
					);
					if (!data.deferCreates) return made;
					return new Promise((resolve) => data.pending.push(() => resolve(made)));
				}
				const params = init.params;
				if (!params) throw new Error(`unexpected POST ${path}`);
				data.posts.push({ path, key: params.path.key, body: init.body });
				if (path === '/api/v1/stories/{key}/rank') {
					// The server's placement against the todo list: the moved story goes before `before`, ranks renumbered.
					const list = answer({ delivery: 'todo' });
					const moved = list.find((s) => s.story.key === params.path.key)!;
					const rest = list.filter((s) => s !== moved);
					const at = init.body.before ? rest.findIndex((s) => s.story.key === init.body.before) : rest.length;
					rest.splice(at, 0, moved);
					const rank = new Map(rest.map((s, i) => [s.story.key, i + 1]));
					data.stories = data.stories.map((s) => (rank.has(s.story.key) ? { ...s, story: { ...s.story, rank: rank.get(s.story.key)! } } : s));
				}
				if (path === '/api/v1/stories/{key}/move') {
					const body = init.body as { to: Story['delivery']; override_reason?: string };
					if (data.refuseMoves && !body.override_reason)
						return Promise.resolve({
							error: { outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: true },
							response: { status: 422 } as Response
						});
					data.stories = data.stories.map((s) => (s.story.key === params.path.key ? { ...s, story: { ...s.story, delivery: body.to } } : s));
				}
				return ok(null);
			}
		}
	};
});
// The URL is reactive (a SvelteURL), so a goto (a filter change) re-renders the page as in the browser.
vi.mock('$app/state', async () => {
	const { SvelteURL } = await import('svelte/reactivity');
	return { page: { url: new SvelteURL('http://localhost/backlog') } };
});
vi.mock('$app/navigation', () => ({
	goto: vi.fn(async (url: URL) => {
		((await import('$app/state')).page.url as URL).href = url.href;
	})
}));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => {
		data.liveFns.push(fn);
	}
}));
vi.mock('#lib/me.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/me.svelte.ts')>()),
	session: await (async () => {
		const { config, me } = await import('#lib/test/fixtures.ts');
		return { me: me({ email: 'a@example.com' }), config: config({ horizon_buckets: ['Now'] }) };
	})(),
	isInteractive: () => true
}));
import Page from './+page.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	Object.assign(data, data.fresh());
	vi.mocked(goto).mockClear();
	go('');
});

/** Sets the backlog URL's query string, as a navigation would. */
function go(search: string) {
	(page.url as URL).href = `http://localhost/backlog${search}`;
	flushSync();
}

async function open(text: string) {
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.textContent).toContain(text);
	});
}
function choose(key: string, label: string) {
	(document.body.querySelector(`button[aria-label="Actions for ${key}"]`) as HTMLButtonElement).click();
	flushSync();
	return [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === label)!;
}
function closeMenu() {
	document.body.querySelector('[role="menu"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
	flushSync();
}
const section = (name: string) => document.body.querySelector(`section[aria-label="${name}"]`) as HTMLElement;
const keys = (el: Element) => [...el.querySelectorAll('li[data-key]')].map((li) => li.getAttribute('data-key'));

describe('backlog page', () => {
	it('lists an epic with no initiative and no stories, and opens New epic with no initiative chosen', async () => {
		await open('Loose epic');
		expect(document.title).toBe('Backlog · Reeve');
		expect(section('No initiative').textContent).toContain('No stories waiting');
		[...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'New epic')!.click();
		flushSync();
		const form = document.body.querySelector('form[aria-label="New epic"]') as HTMLFormElement;
		expect((form.querySelector('select') as HTMLSelectElement).value).toBe('');
	});

	it('opens New story with an epic select defaulting to none', async () => {
		await open('Loose epic');
		[...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'New story')!.click();
		flushSync();
		const form = document.body.querySelector('form[aria-label="New story"]') as HTMLFormElement;
		const selects = [...form.querySelectorAll('select')];
		expect(selects.at(-1)!.value).toBe('');
		expect(form.querySelector('button[type="submit"]')!.textContent).toBe('Create story');
	});

	it('groups by epic: No epic in rank order, an empty initiative says "No epics", an unloaded epic still shows its stories', async () => {
		data.initiatives = [initiative({ key: 'I-1', title: 'Quiet one', horizon: 'Now' })];
		data.stories = [row('S-3', 3, null, false, 'chore', true), row('S-1', 1, 'E-1', false), row('S-2', 2, null, false, 'story', true), row('S-4', 4, 'E-9')];
		await open('Quiet one');
		const none = section('No epic');
		expect(keys(none)).toEqual(['S-2', 'S-3']);
		expect(none.textContent).toContain('2 untriaged');
		expect(section('Quiet one').textContent).toContain('No epics');
		expect(keys(section('No initiative'))).toEqual(['S-1']);
		expect(section('No initiative').textContent).toContain('0 of 1 ready');
		expect(keys(section('Epic not loaded'))).toEqual(['S-4']);
		expect(section('Epic not loaded').textContent).toContain('E-9');
		// The title opens the drawer over the backlog.
		expect(none.querySelector('a.title')!.getAttribute('href')).toBe('/backlog?story=S-2');
	});

	it('ranked: one list in rank order; Move up ranks before the previous row and announces the new place', async () => {
		go('?group=rank');
		data.stories = [row('S-1', 1, null), row('S-2', 2, 'E-1'), row('S-3', 3, null)];
		await open('Title S-3');
		expect(document.body.querySelectorAll('section')).toHaveLength(0);
		expect(keys(document.body)).toEqual(['S-1', 'S-2', 'S-3']);
		expect(document.body.querySelector('li[data-key="S-2"]')!.textContent).toContain('Loose epic');
		(document.body.querySelector('button[aria-label="Actions for S-3"]') as HTMLButtonElement).click();
		flushSync();
		const up = [...document.body.querySelectorAll('[role="menuitem"]')].find((b) => b.textContent === 'Move up') as HTMLButtonElement;
		up.click();
		await vi.waitFor(() => {
			flushSync();
			expect(keys(document.body)).toEqual(['S-1', 'S-3', 'S-2']);
		});
		expect(data.posts).toEqual([{ path: '/api/v1/stories/{key}/rank', key: 'S-3', body: { version: 1, before: 'S-2' } }]);
		expect(document.body.querySelector('[aria-live="polite"]')!.textContent).toBe('S-3 moved to 2 of 3');
		// Focus returns to the moved row's actions button.
		await vi.waitFor(() => expect(document.activeElement?.getAttribute('aria-label')).toBe('Actions for S-3'));
	});

	it('a refused Move to in progress raises the override prompt; Move anyway re-posts with the reason', async () => {
		data.stories = [row('S-1', 1, 'E-1', false)];
		await open('Title S-1');
		(document.body.querySelector('button[aria-label="Actions for S-1"]') as HTMLButtonElement).click();
		flushSync();
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === 'Move to in progress')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog h2')!.textContent).toBe('S-1 is not ready for in progress');
		});
		const why = document.body.querySelector('dialog textarea') as HTMLTextAreaElement;
		why.value = 'Urgent';
		why.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		(why.form as HTMLFormElement).requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
		});
		expect(data.posts.map((p) => p.body)).toEqual([
			{ version: 1, to: 'in_progress' },
			{ version: 1, to: 'in_progress', override_reason: 'Urgent' }
		]);
	});

	it('a row menu offers every column but its own, then Change epic', async () => {
		data.stories = [row('S-1', 1, 'E-1')];
		await open('Title S-1');
		(document.body.querySelector('button[aria-label="Actions for S-1"]') as HTMLButtonElement).click();
		flushSync();
		expect([...document.body.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent)).toEqual([
			'Move up',
			'Move down',
			'Move to in progress',
			'Move to in review',
			'Move to testing',
			'Move to done',
			'Change epic'
		]);
		closeMenu();
	});

	it('Move to done asks for the resolution, then posts it', async () => {
		data.refuseMoves = false;
		data.stories = [row('S-1', 1, 'E-1')];
		await open('Title S-1');
		choose('S-1', 'Move to done').click();
		flushSync();
		expect(document.body.querySelector('dialog h2')!.textContent).toBe('Move S-1 to done');
		[...document.body.querySelectorAll<HTMLButtonElement>('dialog button')].find((b) => b.textContent === 'dropped')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('li[data-key="S-1"]')).toBeNull();
		});
		expect(data.posts.map((p) => p.body)).toEqual([{ version: 1, to: 'done', resolution: 'dropped' }]);
	});

	it('Change epic sends the PATCH with only the epic and reloads', async () => {
		data.epics = [epic({ key: 'E-1', title: 'Loose epic' }), epic({ key: 'E-2', title: 'Other epic' })];
		data.stories = [row('S-1', 1, 'E-1')];
		await open('Title S-1');
		choose('S-1', 'Change epic').click();
		flushSync();
		const form = document.body.querySelector('dialog form[aria-label="Change epic"]') as HTMLFormElement;
		const select = form.querySelector('select') as HTMLSelectElement;
		expect([...select.options].map((o) => o.textContent)).toEqual(['No epic', 'E-1 Loose epic', 'E-2 Other epic']);
		expect(select.value).toBe('E-1');
		select.value = 'E-2';
		select.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		const gets = data.storyGets;
		form.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
			expect(data.storyGets).toBeGreaterThan(gets);
		});
		expect(data.patches).toEqual([{ key: 'S-1', body: { version: 1, epic: 'E-2' } }]);
		await vi.waitFor(() => {
			flushSync();
			expect(keys(document.body.querySelector('h3 a[href="/epics/E-2"]')!.closest('.group')!)).toEqual(['S-1']);
		});
	});

	it('Change epic to No epic sends null', async () => {
		data.stories = [row('S-1', 1, 'E-1')];
		await open('Title S-1');
		choose('S-1', 'Change epic').click();
		flushSync();
		const form = document.body.querySelector('dialog form[aria-label="Change epic"]') as HTMLFormElement;
		const select = form.querySelector('select') as HTMLSelectElement;
		select.value = '';
		select.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		form.requestSubmit();
		await vi.waitFor(() => expect(data.patches).toEqual([{ key: 'S-1', body: { version: 1, epic: null } }]));
	});

	it.each([
		['the next row', ['S-1', 'S-2', 'S-3'], 'S-2', 'Actions for S-3'],
		['the previous row when it was last', ['S-1', 'S-2'], 'S-2', 'Actions for S-1'],
		['the group heading when it was alone', ['S-1'], 'S-1', 'Loose epic E-1']
	])('after Move to in progress, focus goes to %s', async (_, ks, moved, focused) => {
		data.refuseMoves = false;
		data.stories = ks.map((k, i) => row(k, i + 1, 'E-1'));
		await open(`Title ${moved}`);
		(document.body.querySelector(`button[aria-label="Actions for ${moved}"]`) as HTMLButtonElement).click();
		flushSync();
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === 'Move to in progress')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector(`li[data-key="${moved}"]`)).toBeNull();
			const a = document.activeElement!;
			expect(a.getAttribute('aria-label') ?? a.textContent!.replace(/\s+/g, ' ').trim().replace(/ No stories waiting$/, '')).toBe(focused);
		});
	});

	it('a failed first load shows the error with Retry and keeps the header', async () => {
		data.failGets = true;
		await open('Could not load the backlog: boom');
		expect(document.body.querySelector('h1')!.textContent).toContain('Backlog');
		data.failGets = false;
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="alert"] button')].find((b) => b.textContent === 'Retry')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Loose epic');
		});
	});

	it('announces its own result when a live reload overtakes the rank write reload', async () => {
		go('?group=rank');
		data.stories = [row('S-1', 1, null), row('S-2', 2, null), row('S-3', 3, null)];
		await open('Title S-3');
		data.hold = true;
		choose('S-3', 'Move up').click();
		await vi.waitFor(() => expect(data.held).toHaveLength(1)); // the write's reload is waiting
		void data.liveFns[0](); // a live refetch starts after it, so the write's answer will not land
		await vi.waitFor(() => expect(data.held).toHaveLength(2));
		data.held[0]();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[aria-live="polite"]')!.textContent).toBe('S-3 moved to 2 of 3');
		});
		data.held[1]();
		await vi.waitFor(() => {
			flushSync();
			expect(keys(document.body)).toEqual(['S-1', 'S-3', 'S-2']);
		});
	});

	it('by epic: "No epic" is one rank run; Move down past its end ends before the next story', async () => {
		data.stories = [
			row('S-1', 1, null, false, 'bug', true),
			row('S-2', 2, null, false, 'story', true),
			row('S-3', 3, null, false, 'story', true),
			row('S-4', 4, 'E-1')
		];
		await open('Title S-4');
		expect(keys(section('No epic'))).toEqual(['S-1', 'S-2', 'S-3']);
		expect(choose('S-3', 'Move down').disabled).toBe(true); // last of the run
		closeMenu();
		expect(choose('S-1', 'Move up').disabled).toBe(true); // first of the run
		closeMenu();
		// The run ends before S-4 in the full list, so S-2 goes there (after S-3, still inside the run).
		choose('S-2', 'Move down').click();
		await vi.waitFor(() => expect(data.posts).toHaveLength(1));
		expect(data.posts[0]).toEqual({ path: '/api/v1/stories/{key}/rank', key: 'S-2', body: { version: 1, before: 'S-4' } });
	});

	it('rows: the drag handle is out of the tab order and the rank is named', async () => {
		data.stories = [row('S-1', 1, 'E-1')];
		await open('Title S-1');
		const li = document.body.querySelector('li[data-key="S-1"]')!;
		expect(li.querySelector('button.handle')!.getAttribute('tabindex')).toBe('-1');
		expect(li.querySelector('.rank')!.textContent).toBe('rank 1');
	});

	it('ranked: when the last row leaves, focus goes to the page heading', async () => {
		go('?group=rank');
		data.refuseMoves = false;
		data.stories = [row('S-1', 1, null)];
		await open('Title S-1');
		choose('S-1', 'Move to in progress').click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('li[data-key="S-1"]')).toBeNull();
			expect(document.activeElement?.tagName).toBe('H1');
		});
	});

	it('after Move to in progress, focus waits for the reload that removes the row, even when a live reload supersedes the write reload', async () => {
		data.refuseMoves = false;
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, 'E-1')];
		await open('Title S-2');
		data.hold = true;
		choose('S-1', 'Move to in progress').click();
		await vi.waitFor(() => expect(data.held).toHaveLength(1)); // the write's reload
		void data.liveFns[0]();
		await vi.waitFor(() => expect(data.held).toHaveLength(2)); // a live reload after it
		data.held[0](); // the write's reload answers first and is superseded: the row stays
		await new Promise((r) => setTimeout(r, 10));
		flushSync();
		expect(document.body.querySelector('li[data-key="S-1"]')).not.toBeNull();
		data.held[1](); // the live reload lands and removes the row
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('li[data-key="S-1"]')).toBeNull();
			expect(document.activeElement?.getAttribute('aria-label')).toBe('Actions for S-2');
		});
	});

	it.each([
		['story', 'Close'],
		['story', 'Escape'],
		['epic', 'Close'],
		['epic', 'Escape']
	])('keeps the New %s dialog open on %s while its create is pending, then closes it on the answer', async (kind, how) => {
		await open('Loose epic');
		data.deferCreates = true;
		const button = (label: string) => [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === label)!;
		const form = () => document.body.querySelector(`dialog form[aria-label="New ${kind}"]`) as HTMLFormElement | null;
		button(`New ${kind}`).click();
		flushSync();
		const title = form()!.querySelector('input') as HTMLInputElement;
		title.value = 'Sent';
		title.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		form()!.requestSubmit();
		await vi.waitFor(() => expect(data.pending).toHaveLength(1));
		flushSync();
		const dialog = document.body.querySelector('dialog') as HTMLDialogElement;
		const close = dialog.querySelector('button[aria-label="Close"]') as HTMLButtonElement;
		expect(close.disabled).toBe(true);
		if (how === 'Close') close.click();
		else expect(dialog.dispatchEvent(new Event('cancel', { cancelable: true }))).toBe(false);
		flushSync();
		expect(form()).not.toBeNull();
		data.pending[0]();
		await vi.waitFor(() => {
			flushSync();
			expect(form()).toBeNull();
		});
		// The next dialog opens dismissable again.
		button(`New ${kind}`).click();
		flushSync();
		expect((document.body.querySelector('dialog button[aria-label="Close"]') as HTMLButtonElement).disabled).toBe(false);
	});

	it('empties the live region before each announcement, so a repeated one is read again', async () => {
		go('?group=rank');
		data.stories = [row('S-1', 1, null), row('S-2', 2, null), row('S-3', 3, null)];
		await open('Title S-3');
		const region = document.body.querySelector('[aria-live="polite"]')!;
		choose('S-3', 'Move up').click();
		await vi.waitFor(() => {
			flushSync();
			expect(region.textContent).toBe('S-3 moved to 2 of 3');
		});
		const seen: string[] = [];
		const watch = new MutationObserver(() => seen.push(region.textContent ?? ''));
		watch.observe(region, { childList: true, characterData: true, subtree: true });
		await vi.waitFor(() => expect(document.activeElement?.getAttribute('aria-label')).toBe('Actions for S-3'));
		choose('S-3', 'Move down').click();
		await vi.waitFor(() => {
			flushSync();
			expect(region.textContent).toBe('S-3 moved to 3 of 3');
		});
		watch.disconnect();
		expect(seen[0]).toBe('');
		expect(seen.at(-1)).toBe('S-3 moved to 3 of 3');
	});

	it('shows one notice at a time: a write error hides a failed refetch', async () => {
		data.stories = [row('S-1', 1, 'E-1')];
		await open('Title S-1');
		data.failGets = true;
		void data.liveFns[0]();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Could not reload the backlog');
		});
		data.failGets = false;
		data.failPatch = true;
		choose('S-1', 'Change epic').click();
		flushSync();
		const form = document.body.querySelector('dialog form[aria-label="Change epic"]') as HTMLFormElement;
		const select = form.querySelector('select') as HTMLSelectElement;
		select.value = '';
		select.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		form.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('patch failed');
		});
		expect(document.body.textContent).not.toContain('Could not reload the backlog');
	});

	it('says "Nothing waiting" when there is nothing at all', async () => {
		data.epics = [];
		await open('Nothing waiting');
	});
});

describe('backlog filters (the server filters, kb/ADR-0021 R1)', () => {
	const menuLabels = (key: string) => {
		(document.body.querySelector(`button[aria-label="Actions for ${key}"]`) as HTMLButtonElement).click();
		flushSync();
		const labels = [...document.body.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent);
		closeMenu();
		return labels;
	};
	const lastQuery = () => data.queries.at(-1);
	const count = () => document.body.querySelector('h1 .count')?.textContent;

	it('the plain backlog asks only for the todo list; a filter asks for the todo list and the filtered rows together', async () => {
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, null)];
		await open('Title S-1');
		expect(data.queries).toEqual([{ delivery: 'todo' }]);
		data.queries = [];
		go('?epic=E-1');
		await vi.waitFor(() => {
			flushSync();
			expect(keys(document.body)).toEqual(['S-1']);
		});
		expect(data.queries).toEqual([{ delivery: 'todo' }, { delivery: 'todo', epic: 'E-1' }]);
		expect(count()).toBe('1 of 2');
	});

	it('a filtered Move down places against the full todo list, before the hidden story that follows', async () => {
		go('?epic=E-1');
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, null), row('S-3', 3, 'E-1'), row('S-4', 4, null)];
		await open('Title S-3');
		expect(keys(document.body)).toEqual(['S-1', 'S-3']);
		choose('S-1', 'Move down').click();
		await vi.waitFor(() => expect(data.posts).toHaveLength(1));
		expect(data.posts[0]).toEqual({ path: '/api/v1/stories/{key}/rank', key: 'S-1', body: { version: 1, before: 'S-4' } });
	});

	it.each([
		['Open', '?state=open'],
		['All', '?state=all'],
		['Relevance', '?q=title&state=open&view=relevance']
	])('offers no rank controls in %s', async (_, search) => {
		go(search);
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, 'E-1', true, 'story', false, { delivery: 'in_progress' })];
		await open('Title S-2');
		expect(menuLabels('S-1')).not.toContain('Move up');
		expect(menuLabels('S-1')).not.toContain('Move down');
		expect(menuLabels('S-1')).toContain('Move to in progress');
		expect(document.body.querySelector('button.handle')).toBeNull();
	});

	it('in Open the rank cell shows the state of a row that is not todo, and the count says stories', async () => {
		go('?state=open');
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, 'E-1', true, 'story', false, { delivery: 'in_progress' }), row('S-3', 3, 'E-1', true, 'story', false, { delivery: 'done' })];
		await open('Title S-2');
		expect(keys(document.body)).toEqual(['S-1', 'S-2']);
		expect(document.body.querySelector('li[data-key="S-1"] .rank')!.textContent).toBe('rank 1');
		expect(document.body.querySelector('li[data-key="S-2"] .rank')!.textContent).toBe('in progress');
		expect(count()).toBe('2 stories');
	});

	it('an answer for an earlier query that lands after the current one leaves the current rows', async () => {
		data.stories = [row('S-1', 1, null, true, 'story', false, { title: 'Apple' }), row('S-2', 2, null, true, 'story', false, { title: 'Banana' })];
		await open('Apple');
		data.hold = true;
		go('?q=apple');
		await vi.waitFor(() => expect(data.held).toHaveLength(2));
		// While the rows are for another query, they offer no action.
		flushSync();
		expect(document.body.querySelector('ol.rows')!.getAttribute('aria-busy')).toBe('true');
		go('?q=banana');
		await vi.waitFor(() => expect(data.held).toHaveLength(4));
		data.held[2]();
		data.held[3]();
		await vi.waitFor(() => {
			flushSync();
			expect(keys(document.body)).toEqual(['S-2']);
		});
		data.held[0]();
		data.held[1]();
		await new Promise((r) => setTimeout(r, 10));
		flushSync();
		expect(keys(document.body)).toEqual(['S-2']);
		expect(document.body.querySelector('ol.rows')!.getAttribute('aria-busy')).toBe('false');
		// A live refetch asks with the current URL's query.
		data.hold = false;
		void data.liveFns[0]();
		await vi.waitFor(() => expect(lastQuery()).toEqual({ q: 'banana', delivery: 'todo' }));
	});

	it('a failed refetch of the same query keeps the rows with the stale notice; a failed new query shows the error', async () => {
		go('?epic=E-1');
		data.stories = [row('S-1', 1, 'E-1')];
		await open('Title S-1');
		data.failFiltered = true;
		void data.liveFns[0]();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Could not reload the backlog: filter failed');
		});
		expect(keys(document.body)).toEqual(['S-1']);
		go('?epic=E-1&kind=bug');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Could not load the backlog: filter failed');
		});
		expect(keys(document.body)).toEqual([]);
	});

	it('by epic under a filter omits epics and initiatives without matches and counts the matches', async () => {
		go('?kind=bug');
		data.initiatives = [initiative({ key: 'I-1', title: 'Quiet one', horizon: 'Now' })];
		data.epics = [epic({ key: 'E-1', title: 'Loose epic' }), epic({ key: 'E-2', title: 'Empty epic' }), epic({ key: 'E-3', title: 'Quiet epic', initiative: 'I-1' })];
		data.stories = [row('S-1', 1, 'E-1', true, 'bug'), row('S-2', 2, 'E-1', true, 'bug'), row('S-3', 3, 'E-2'), row('S-4', 4, 'E-3')];
		await open('Title S-2');
		expect(section('Quiet one')).toBeNull();
		expect(document.body.querySelector('a[href="/epics/E-2"]')).toBeNull();
		expect(document.body.querySelector('a[href="/epics/E-1"]')).not.toBeNull();
		expect(section('No initiative').textContent).toContain('2 matches');
		expect(section('No epic')).toBeNull();
	});

	it('under text: matches are highlighted, a description match is tagged, and the search-all link is offered', async () => {
		go('?q=zebra&state=open&view=relevance');
		data.stories = [row('S-1', 1, null, true, 'story', false, { title: 'A zebra crossing' }), row('S-2', 2, null, true, 'story', false, { description: 'About a Zebra' })];
		await open('A zebra crossing');
		expect(document.body.querySelector('li[data-key="S-1"] mark')!.textContent).toBe('zebra');
		expect(document.body.querySelector('li[data-key="S-1"]')!.textContent).not.toContain('matched');
		expect(document.body.querySelector('li[data-key="S-2"]')!.textContent).toContain('matched description');
		const link = [...document.body.querySelectorAll('a')].find((a) => a.textContent?.includes('Search all items'))!;
		expect(link.getAttribute('href')).toBe('/search?q=zebra');
		expect(lastQuery()).toEqual({ q: 'zebra', done: 'none', sort: 'relevance' });
	});

	it('no match: says so, offers Clear filters and the search-all link; Clear filters keeps text, state and view', async () => {
		go('?q=nothing&state=open&view=rank&kind=bug');
		data.stories = [row('S-1', 1, null)];
		await open('No stories match');
		expect([...document.body.querySelectorAll('a')].some((a) => a.getAttribute('href') === '/search?q=nothing')).toBe(true);
		[...document.body.querySelectorAll<HTMLButtonElement>('.empty button')].find((b) => b.textContent?.trim() === 'Clear filters')!.click();
		await vi.waitFor(() => expect(vi.mocked(goto)).toHaveBeenCalled());
		const [url, opts] = vi.mocked(goto).mock.calls.at(-1)!;
		// The filters go; the text, the state and the view stay (the chip is what drops the text).
		expect(String(url)).toBe('http://localhost/backlog?q=nothing&state=open&view=rank');
		expect(opts).toEqual({ replace: true, reset: false }); // SvelteKit 3: reset false keeps focus and scroll
	});

	it('the text chip drops the text and relevance with it', async () => {
		go('?q=zebra&state=open&view=relevance&kind=bug');
		await open('No stories match');
		(document.body.querySelector('button[aria-label^="Remove the text filter"]') as HTMLButtonElement).click();
		await vi.waitFor(() => expect(String(page.url)).toBe('http://localhost/backlog?state=open&kind=bug'));
	});

	it('Filters (n) is open on load when a filter is set; labels come from the todo list, the rows and the selection', async () => {
		go('?state=all&label=gone');
		data.stories = [row('S-1', 1, null, true, 'story', false, { labels: ['ux'] }), row('S-2', 2, null, true, 'story', false, { labels: ['old'], delivery: 'done' })];
		await open('No stories match');
		const toggle = [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim().startsWith('Filters'))!;
		expect(toggle.textContent!.trim()).toBe('Filters (1)');
		expect(toggle.getAttribute('aria-expanded')).toBe('true');
		go('?state=all');
		await vi.waitFor(() => {
			flushSync();
			expect(keys(document.body)).toEqual(['S-1', 'S-2']);
		});
		go('?state=all&label=gone');
		await vi.waitFor(() => {
			flushSync();
			const select = [...document.body.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith('Label'))!.querySelector('select')!;
			expect([...select.options].map((o) => o.value)).toEqual(['', 'gone', 'old', 'ux']);
		});
	});

	it('the plain backlog opens with Filters closed', async () => {
		await open('Loose epic');
		const toggle = [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim().startsWith('Filters'))!;
		expect(toggle.textContent!.trim()).toBe('Filters');
		expect(toggle.getAttribute('aria-expanded')).toBe('false');
	});

	it('after a move under Open the row is still shown: focus returns to its actions trigger', async () => {
		go('?state=open');
		data.refuseMoves = false;
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, 'E-1')];
		await open('Title S-2');
		choose('S-1', 'Move to in progress').click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('li[data-key="S-1"] .rank')!.textContent).toBe('in progress');
			expect(document.activeElement?.getAttribute('aria-label')).toBe('Actions for S-1');
		});
	});

	it('after an epic change that hides the row under an epic filter, focus goes to the next shown row', async () => {
		go('?epic=E-1');
		data.epics = [epic({ key: 'E-1', title: 'Loose epic' }), epic({ key: 'E-2', title: 'Other epic' })];
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, 'E-2'), row('S-3', 3, 'E-1')];
		await open('Title S-3');
		choose('S-1', 'Change epic').click();
		flushSync();
		const form = document.body.querySelector('dialog form[aria-label="Change epic"]') as HTMLFormElement;
		const select = form.querySelector('select') as HTMLSelectElement;
		select.value = 'E-2';
		select.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		form.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('li[data-key="S-1"]')).toBeNull();
			expect(document.activeElement?.getAttribute('aria-label')).toBe('Actions for S-3');
		});
	});

	it('offers no rank controls by relevance in Todo; the same rows ranked offer Move up', async () => {
		data.stories = [row('S-1', 1, 'E-1'), row('S-2', 2, 'E-1')];
		go('?q=title&view=relevance');
		await open('Title S-2');
		expect(menuLabels('S-2')).not.toContain('Move up');
		expect(document.body.querySelector('button.handle')).toBeNull();
		go('?q=title&view=rank');
		await vi.waitFor(() => {
			flushSync();
			expect(lastQuery()).toEqual({ q: 'title', delivery: 'todo' });
			expect(document.body.querySelector('ol.rows')!.getAttribute('aria-busy')).toBe('false');
		});
		expect(menuLabels('S-2')).toContain('Move up');
		expect(document.body.querySelector('button.handle')).not.toBeNull();
	});

	it("by relevance the rows are one flat list in the server's relevance order, not rank order", async () => {
		go('?q=apple&state=open&view=relevance');
		data.stories = [
			row('S-1', 1, 'E-1', true, 'story', false, { title: 'Pears', description: 'with apple' }),
			row('S-2', 2, null, true, 'story', false, { title: 'Apple pie' })
		];
		await open('Apple pie');
		expect(document.body.querySelectorAll('section')).toHaveLength(0);
		expect(keys(document.body)).toEqual(['S-2', 'S-1']);
	});

	it('announces the count after a filter change, not after the first load', async () => {
		data.stories = [row('S-1', 1, 'E-1', true, 'bug'), row('S-2', 2, null)];
		await open('Title S-2');
		const live = () => document.body.querySelector('[aria-live="polite"]')!.textContent;
		expect(live()).toBe('');
		go('?kind=bug');
		await vi.waitFor(() => {
			flushSync();
			expect(live()).toBe('1 of 2 stories');
		});
		go('?kind=bug&state=open');
		await vi.waitFor(() => {
			flushSync();
			expect(live()).toBe('1 story');
		});
	});

	it('announces the count of the answer that recovers from a failed new query', async () => {
		data.stories = [row('S-1', 1, 'E-1', true, 'bug'), row('S-2', 2, null)];
		await open('Title S-2');
		data.failFiltered = true;
		go('?kind=bug');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Could not load the backlog: filter failed');
		});
		data.failFiltered = false;
		[...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Retry')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[aria-live="polite"]')!.textContent).toBe('1 of 2 stories');
		});
	});

	it('while a new query is held, a move posts nothing and the old rows keep their own decorations', async () => {
		data.refuseMoves = false;
		data.stories = [row('S-1', 1, null, true, 'story', false, { title: 'Apple' }), row('S-2', 2, null, true, 'story', false, { title: 'Banana' })];
		go('?q=apple');
		await open('Apple');
		expect(count()).toBe('1 of 2');
		data.hold = true;
		go('?q=banana');
		await vi.waitFor(() => expect(data.held).toHaveLength(2));
		flushSync();
		choose('S-1', 'Move to in progress').click();
		flushSync();
		await new Promise((r) => setTimeout(r, 10));
		expect(data.posts).toEqual([]);
		closeMenu();
		// Still the rows for "apple": highlighted as such, counted as such, untagged.
		expect(document.body.querySelector('li[data-key="S-1"] mark')?.textContent).toBe('Apple');
		expect(document.body.querySelector('li[data-key="S-1"]')!.textContent).not.toContain('matched');
		expect(count()).toBe('1 of 2');
		data.held.forEach((f) => f());
		await vi.waitFor(() => {
			flushSync();
			expect(keys(document.body)).toEqual(['S-2']);
		});
		expect(document.body.querySelector('li[data-key="S-2"] mark')?.textContent).toBe('Banana');
	});

	it('the chip and Clear filters hand focus to the Filters button once they are gone', async () => {
		go('?q=zebra&kind=bug');
		data.stories = [row('S-1', 1, null, true, 'bug', false, { title: 'zebra' })];
		await open('zebra');
		const toggle = () => document.body.querySelector('button[data-filters-toggle]') as HTMLButtonElement;
		const clear = [...document.body.querySelectorAll('.panel button')].find((b) => b.textContent?.trim() === 'Clear filters') as HTMLButtonElement;
		clear.focus();
		clear.click();
		await vi.waitFor(() => expect(document.activeElement).toBe(toggle()));
		expect(String(page.url)).toBe('http://localhost/backlog?q=zebra');
		const chip = document.body.querySelector('button[aria-label^="Remove the text filter"]') as HTMLButtonElement;
		chip.focus();
		chip.click();
		await vi.waitFor(() => expect(document.activeElement).toBe(toggle()));
		expect(String(page.url)).toBe('http://localhost/backlog');
	});

	it('the Filters panel is always in the DOM right after its trigger, hidden while closed', async () => {
		await open('Loose epic');
		const toggle = document.body.querySelector('button[data-filters-toggle]') as HTMLButtonElement;
		const panel = document.getElementById(toggle.getAttribute('aria-controls')!)!;
		expect(panel).not.toBeNull();
		expect(toggle.nextElementSibling).toBe(panel);
		expect(panel.hidden).toBe(true);
		toggle.click();
		flushSync();
		expect(panel.hidden).toBe(false);
		expect(toggle.getAttribute('aria-expanded')).toBe('true');
	});

	it('the chip ellipsises its text inside, the × outside it', async () => {
		go('?q=zebra');
		await open('No todo stories match');
		const chip = document.body.querySelector('button[aria-label^="Remove the text filter"]')!;
		expect(chip.querySelector('.text')!.textContent).toBe('“zebra”');
		expect(chip.querySelector('.x')!.textContent).toBe('×');
	});

	it('the Todo empty state under text offers the open stories', async () => {
		go('?q=zzz');
		data.stories = [row('S-1', 1, null)];
		await open('No todo stories match');
		expect(document.title).toBe('Backlog: “zzz” · Reeve');
		[...document.body.querySelectorAll<HTMLButtonElement>('.empty button')].find((b) => b.textContent?.trim() === 'Show open stories')!.click();
		await vi.waitFor(() => expect(String(page.url)).toBe('http://localhost/backlog?q=zzz&state=open'));
	});

	it('the empty state names a key that matched nothing, and says "No stories" when nothing is filtered', async () => {
		go('?q=ABC-99&state=open');
		await open('No item ABC-99');
		expect(document.body.textContent).not.toContain('Show open stories');
		go('?state=open');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('.empty')!.textContent!.trim()).toBe('No stories');
		});
	});

	it('under All a done row shows its resolution, epics show their story count and the footer leaves out done work', async () => {
		go('?q=title&state=all');
		data.stories = [
			row('S-1', 1, 'E-1', false),
			row('S-2', 2, 'E-1', true, 'story', false, { delivery: 'done', resolution: 'dropped' })
		];
		await open('Title S-2');
		expect(document.body.querySelector('li[data-key="S-2"] .end')!.textContent).toContain('dropped');
		expect(document.body.querySelector('li[data-key="S-1"] .end')!.textContent).toContain('no criteria');
		expect(section('No initiative').textContent).toContain('2 matches');
		expect(document.body.querySelector('.footer')!.textContent).toContain('including epics and initiatives, for');
		go('?state=all');
		await vi.waitFor(() => {
			flushSync();
			expect(section('No initiative').textContent).toContain('2 stories');
		});
		expect(document.body.querySelector('.footer')).toBeNull();
	});

	it('the Epic select lists only the chosen initiative\'s epics, and the chosen epic', async () => {
		data.initiatives = [initiative({ key: 'I-1', title: 'Hosting', horizon: 'Now' })];
		data.epics = [epic({ key: 'E-1', title: 'Loose epic' }), epic({ key: 'E-2', title: 'Move', initiative: 'I-1' })];
		go('?initiative=I-1');
		await open('No stories match');
		const epicSelect = () => [...document.body.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith('Epic'))!.querySelector('select')!;
		expect([...epicSelect().options].map((o) => o.value)).toEqual(['', 'E-2']);
		go('?initiative=I-1&epic=E-1');
		await vi.waitFor(() => {
			flushSync();
			expect([...epicSelect().options].map((o) => o.value)).toEqual(['', 'E-1', 'E-2']);
		});
		go('');
		await vi.waitFor(() => {
			flushSync();
			expect([...epicSelect().options].map((o) => o.value)).toEqual(['', 'E-1', 'E-2']);
		});
	});
});
