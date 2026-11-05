// @vitest-environment jsdom
// The backlog: the ranked todo list and the only epic index the app has. Every epic is listed, including one
// with no initiative and no waiting stories; an initiative with no epics says so; a story whose epic is not loaded is
// still shown; Move up ranks through the shared rank write and announces the new position.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Epic, Initiative, Story, StoryWithReadiness } from '#lib/api/types.ts';
import { epic, initiative, withReadiness } from '#lib/test/fixtures.ts';

/** `untriaged` puts the server's untriaged reason on the row (the page never derives it from kind and epic). */
const row = (key: string, rank: number, epic: string | null, ready = true, kind: Story['kind'] = 'story', untriaged = false) =>
	withReadiness(
		{ key, title: `Title ${key}`, kind, epic, rank },
		untriaged ? [{ reason: 'untriaged' }] : ready ? [] : [{ reason: 'no_criteria' }]
	);

const data = vi.hoisted(() => {
	const loose: Epic = { key: 'E-1', title: 'Loose epic', initiative: null, brief: '', version: 1, discovery_complete: null, external_ref: null };
	const fresh = () => ({
		stories: [] as StoryWithReadiness[],
		epics: [{ ...loose }],
		initiatives: [] as Initiative[],
		posts: [] as { path: string; key: string; body: unknown }[],
		patches: [] as { key: string; body: { version: number; epic?: string | null } }[],
		storyGets: 0,
		failPatch: false,
		url: new URL('http://localhost/backlog'),
		failGets: false,
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
vi.mock('#lib/api/client.ts', async (orig) => {
	const fixtures = await import('#lib/test/fixtures.ts');
	const ok = (value: unknown) => Promise.resolve({ data: { outcome: 'ok', value }, response: { status: 200 } as Response });
	return {
		...(await orig<typeof import('#lib/api/client.ts')>()),
		api: {
			GET: (path: string) => {
				if (data.failGets)
					return Promise.resolve({ error: { outcome: 'error', code: 'internal', message: 'boom' }, response: { status: 500 } as Response });
				if (data.hold && path.endsWith('/stories')) {
					const asked = data.stories;
					return new Promise((resolve) => data.held.push(() => resolve(ok(asked))));
				}
				if (path.endsWith('/stories')) data.storyGets++;
				return ok(path.endsWith('/stories') ? data.stories : path.endsWith('/epics') ? data.epics : data.initiatives);
			},
			PATCH: (_path: string, init: { params: { path: { key: string } }; body: { version: number; epic?: string | null } }) => {
				const key = init.params.path.key;
				if (data.failPatch)
					return Promise.resolve({ error: { outcome: 'error', code: 'internal', message: 'patch failed' }, response: { status: 500 } as Response });
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
					// The server's placement: the moved story goes before `before`, ranks renumbered.
					const list = data.stories;
					const moved = list.find((s) => s.story.key === params.path.key)!;
					const rest = list.filter((s) => s !== moved);
					const at = init.body.before ? rest.findIndex((s) => s.story.key === init.body.before) : rest.length;
					rest.splice(at, 0, moved);
					data.stories = rest.map((s, i) => ({ ...s, story: { ...s.story, rank: i + 1 } }));
				}
				if (path === '/api/v1/stories/{key}/move') {
					if (data.refuseMoves && !(init.body as { override_reason?: string }).override_reason)
						return Promise.resolve({
							error: { outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: true },
							response: { status: 422 } as Response
						});
					data.stories = data.stories.filter((s) => s.story.key !== params.path.key);
				}
				return ok(null);
			}
		}
	};
});
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return data.url;
		}
	}
}));
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
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
});

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

	it('groups by epic: untriaged first, an empty initiative says "No epics", an unloaded epic still shows its stories', async () => {
		data.initiatives = [initiative({ key: 'I-1', title: 'Quiet one', horizon: 'Now' })];
		data.stories = [row('S-3', 3, null, true, 'chore'), row('S-1', 1, 'E-1', false), row('S-2', 2, null, false, 'story', true), row('S-4', 4, 'E-9')];
		await open('Quiet one');
		const none = section('No epic');
		expect(keys(none)).toEqual(['S-2', 'S-3']);
		expect(none.textContent).toContain('1 untriaged');
		expect(section('Quiet one').textContent).toContain('No epics');
		expect(keys(section('No initiative'))).toEqual(['S-1']);
		expect(section('No initiative').textContent).toContain('0 of 1 ready');
		expect(keys(section('Epic not loaded'))).toEqual(['S-4']);
		expect(section('Epic not loaded').textContent).toContain('E-9');
		// The title opens the drawer over the backlog.
		expect(none.querySelector('a.title')!.getAttribute('href')).toBe('/backlog?story=S-2');
	});

	it('ranked: one list in rank order; Move up ranks before the previous row and announces the new place', async () => {
		data.url = new URL('http://localhost/backlog?group=rank');
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
		data.url = new URL('http://localhost/backlog?group=rank');
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

	it('by epic: rows rank within their run of "No epic"; Move down past a run ends before the next story', async () => {
		data.stories = [
			row('S-1', 1, null, true, 'bug'),
			row('S-2', 2, null, false, 'story', true),
			row('S-3', 3, null, false, 'story', true),
			row('S-4', 4, 'E-1')
		];
		await open('Title S-4');
		expect(keys(section('No epic'))).toEqual(['S-2', 'S-3', 'S-1']);
		expect(choose('S-3', 'Move down').disabled).toBe(true); // last of the untriaged run
		closeMenu();
		expect(choose('S-1', 'Move up').disabled).toBe(true); // alone in its run
		closeMenu();
		// The run [S-2, S-3] ends before S-4 in the full list, so S-2 goes there (after S-3, still inside its run).
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
		data.url = new URL('http://localhost/backlog?group=rank');
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
		data.url = new URL('http://localhost/backlog?group=rank');
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
