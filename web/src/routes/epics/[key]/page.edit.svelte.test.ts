// @vitest-environment jsdom
// The epic page's details editor: only changed fields are sent against the version the editor started from, a reload or
// live event never wipes the draft, detaching sends null (not absent), a conflict keeps the user's fields on the reloaded
// epic, errors stay in the form, Cancel clears them and returns focus, and a page key change closes the editor.
// Stories and status: each story row unfolds its criteria from a chevron and rings when someone else just changed it, the
// status strip counts the stories the server holds back for criteria, progress and the blocking decisions on the epic
// and on its initiative (never another epic's), the story form opens from "New story" in a dialog, and the decisions are
// one split list whose fold unfolds the accepted ones after the blocking ones. Rows open independently and keep an
// unsaved criteria draft through folding, show Bug and Chore, an alias URL counts under the returned key, and the New
// story dialog cannot be dismissed while its create is pending.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { session } from '#lib/me.svelte.ts';
import { live as liveState } from '#lib/events.svelte.ts';
import type { Epic, Initiative, Reason, RequiredDecision, Story, StoryWithReadiness } from '#lib/api/types.ts';
import { activity, config, decision, initiative, withReadiness } from '#lib/test/fixtures.ts';
import type { FakePage } from '#lib/test/net.ts';
import Page from './+page.svelte';

// Read beside this file (a `new URL(…, import.meta.url)` is rewritten by Vite to an asset URL; a CSS import is empty).
const appCss = readFileSync(resolve(import.meta.dirname, '../../../app.css'), 'utf8');

const live = vi.hoisted(() => ({ fns: [] as (() => unknown)[] }));
const fireLive = () => Promise.all(live.fns.map((f) => f()));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => {
		live.fns.push(fn);
	}
}));
const route = vi.hoisted(() => ({ fake: null as null | FakePage }));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	route.fake = fakePage({ url: '/epics/ABC-5', params: { key: 'ABC-5' } });
	return { page: route.fake.page };
});

type Patched = { version: number; title?: string; initiative?: string | null };

const net = vi.hoisted(() => {
	const fresh = (): Epic => ({ key: 'ABC-5', title: 'Login', brief: '', version: 3, initiative: 'ABC-1', discovery_complete: null, external_ref: null, urgent_mark: null, urgent_sources: [] });
	// Built by hand: the fixtures cannot be imported into a hoisted block.
	const ini = (key: string, title: string): Initiative => ({ key, title, brief: '', horizon: 'Now', version: 1, urgent_mark: null, urgent_sources: [] });
	const freshInitiatives = () => [ini('ABC-1', 'Orion'), ini('ABC-2', 'Other')];
	const s = {
		fresh,
		freshInitiatives,
		epic: fresh(),
		initiatives: freshInitiatives(),
		patches: [] as Patched[],
		urgentFail: false,
		urgentWrites: [] as { method: string; path: string; body: unknown }[],
		stories: [] as StoryWithReadiness[],
		decisions: [] as RequiredDecision[],
		initiativeDecisions: [] as RequiredDecision[],
		initiativeReads: [] as string[],
		storyPosts: [] as unknown[],
		/** GETs of ABC-6, the epic the New story tests move to: a create's refresh reads it once the create answers. */
		otherReads: 0,
		storyGate: null as null | Promise<void>,
		/** While set, each create POST answers only when its queue entry is called: true succeeds, false fails. */
		deferCreates: false,
		createQueue: [] as ((ok: boolean) => void)[],
		decisionPosts: [] as { on: string; title: string }[],
		/** Each decision create POST answers only when its queue entry is called: true succeeds, false fails. */
		decisionQueue: [] as ((ok: boolean) => void)[],
		patchAnswer: null as null | { status: number; body: unknown },
		gate: null as null | Promise<void>
	};
	return s;
});
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure, json } = await import('#lib/test/net.ts');
	const { decision, story } = await import('#lib/test/fixtures.ts');
	const s = net;
	installFetch(async ({ method, url, path, body }) => {
		if ((method === 'POST' || method === 'DELETE') && path === '/api/v1/epics/ABC-5/urgent') {
			s.urgentWrites.push({ method, path, body });
			if (s.urgentFail) return failure('boom');
			const mark = method === 'POST' ? { at: '2026-11-09T08:00:00Z', by: 'dan@example.com', reason: (body as { reason: string | null }).reason, expires_at: '2099-01-01T00:00:00Z', active: true, done: false } : null;
			s.epic = { ...s.epic, urgent_mark: mark, urgent_sources: mark ? [{ key: 'ABC-5', item: 'epic' }] : [] };
			return s.epic;
		}
		if (method === 'GET' && path === '/api/v1/epics/ABC-5') return s.epic;
		if (method === 'GET' && path === '/api/v1/epics/JIRA-55') return s.epic;
		if (method === 'POST' && path === '/api/v1/stories') {
			const b = body as { title: string };
			s.storyPosts.push(b);
			if (s.storyGate) await s.storyGate;
			if (s.deferCreates) {
				const ok = await new Promise<boolean>((r) => s.createQueue.push(r));
				if (!ok) return failure('boom');
			}
			return story({ key: 'ABC-99', title: b.title, epic: 'ABC-5' });
		}
		if (method === 'POST' && path === '/api/v1/decisions') {
			const b = body as { on: string; title: string; decider: string };
			s.decisionPosts.push(b);
			const ok = await new Promise<boolean>((r) => s.decisionQueue.push(r));
			if (!ok) return failure('boom');
			return decision({ id: 77, on: b.on, on_type: 'epic', title: b.title, decider: b.decider });
		}
		if (method === 'GET' && path === '/api/v1/epics/ABC-6') {
			s.otherReads++;
			return { ...s.epic, key: 'ABC-6', title: 'Elsewhere' };
		}
		if (method === 'GET' && path === '/api/v1/initiatives') return s.initiatives;
		if (method === 'GET' && path === '/api/v1/stories') return s.stories;
		if (method === 'GET' && path === '/api/v1/decisions') {
			const ini = url.searchParams.get('initiative');
			if (ini) s.initiativeReads.push(ini);
			return ini ? s.initiativeDecisions : s.decisions;
		}
		if (method === 'PATCH' && path === '/api/v1/epics/ABC-5') {
			const b = body as Patched;
			s.patches.push(b);
			if (s.gate) await s.gate;
			if (s.patchAnswer) return json(s.patchAnswer.body, s.patchAnswer.status);
			s.epic = { ...s.epic, ...b, version: s.epic.version + 1 };
			return s.epic;
		}
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	live.fns.length = 0;
	document.body.innerHTML = '';
	net.epic = net.fresh();
	net.initiatives = net.freshInitiatives();
	net.patches.length = 0;
	net.urgentWrites.length = 0;
	net.urgentFail = false;
	net.stories = [];
	net.decisions = [];
	net.initiativeDecisions = [];
	net.initiativeReads.length = 0;
	net.storyPosts.length = 0;
	net.otherReads = 0;
	net.storyGate = null;
	net.deferCreates = false;
	net.createQueue.length = 0;
	net.decisionPosts.length = 0;
	net.decisionQueue.length = 0;
	net.patchAnswer = null;
	net.gate = null;
	session.config = null;
	route.fake!.reset();
});

const btn = (name: string) => [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === name) as HTMLButtonElement;
const detailsForm = () => document.body.querySelector('form[aria-label="Epic details"]') as HTMLFormElement;
const label = (name: string) => [...detailsForm().querySelectorAll('label')].find((x) => x.textContent?.trim().startsWith(name))!;
const title = () => label('Title').querySelector('input') as HTMLInputElement;
const parent = () => label('Initiative').querySelector('select') as HTMLSelectElement;
function type(el: HTMLInputElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
}
function choose(v: string) {
	parent().value = v;
	parent().dispatchEvent(new Event('change', { bubbles: true }));
	flushSync();
}
async function open() {
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.textContent).toContain('Edit details');
	});
}
async function startEditing() {
	await open();
	btn('Edit details').click();
	flushSync();
}
const crumbLink = () => document.body.querySelector('nav.crumb a[href^="/initiatives/"]');
const detailsAlert = () => detailsForm()?.querySelector('[role="alert"]')?.textContent ?? '';
const notice = () => document.body.querySelector('.notice')?.textContent ?? '';
const conflict = { status: 409, body: { outcome: 'error', code: 'conflict', message: 'x', current: 4 } };
const closed = () =>
	vi.waitFor(() => {
		flushSync();
		expect(detailsForm()).toBeNull();
	});

describe('epic page: edit details', () => {
	it('offers none, every initiative and the current parent in the select', async () => {
		net.initiatives = [initiative({ key: 'ABC-2', title: 'Other' })]; // the list lags: ABC-1 is not in it
		await startEditing();
		expect([...parent().options].map((o) => o.value)).toEqual(['', 'ABC-2', 'ABC-1']);
		expect(parent().value).toBe('ABC-1');
	});

	it("lists the initiatives in the planning page's order, not the server's", async () => {
		session.config = config({ horizon_buckets: ['Now', 'Next'] });
		net.initiatives = [
			initiative({ key: 'ABC-1', title: 'Orion', horizon: 'Next' }),
			initiative({ key: 'ABC-2', title: 'Other', horizon: 'Now' }),
			initiative({ key: 'ABC-3', title: 'Atlas', horizon: 'Now' })
		];
		await startEditing();
		expect([...parent().options].map((o) => o.value)).toEqual(['', 'ABC-3', 'ABC-2', 'ABC-1']);
	});

	it('sends only the changed title with the version the editor started from', async () => {
		await startEditing();
		type(title(), ' Sign in ');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 3, title: 'Sign in' });
		await closed();
	});

	it('moving the initiative sends the key and leaves the epic key and URL alone', async () => {
		await startEditing();
		choose('ABC-2');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 3, initiative: 'ABC-2' });
		await closed();
		await fireLive();
		flushSync();
		expect(detailsForm()).toBeNull();
		expect(route.fake!.params.get('key')).toBe('ABC-5');
		expect(crumbLink()?.getAttribute('href')).toBe('/initiatives/ABC-2');
	});

	it('detaching sends null, not an absent field, and shows the banner', async () => {
		await startEditing();
		choose('');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 3, initiative: null });
		expect('initiative' in net.patches[0]).toBe(true);
		await closed();
		expect(crumbLink()).toBeNull();
		const banner = document.body.querySelector('.banner')!;
		expect(banner.getAttribute('role')).toBe('status');
		expect(banner.textContent).toContain('This epic belongs to no initiative.');
	});

	it('the banner button opens the editor on an orphan epic', async () => {
		net.epic = { ...net.epic, initiative: null };
		await open();
		btn('Choose an initiative').click();
		flushSync();
		expect(parent().value).toBe('');
		expect(btn('Choose an initiative')).toBeUndefined();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(parent());
		});
		choose('ABC-1');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 3, initiative: 'ABC-1' });
	});

	it('opening the editor focuses its first field', async () => {
		await startEditing();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(title());
		});
	});

	it('a successful save returns focus to Edit details', async () => {
		await startEditing();
		type(title(), 'Login two');
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsForm()).toBeNull();
			expect(document.activeElement).toBe(btn('Edit details'));
		});
		expect(net.patches).toEqual([{ version: 3, title: 'Login two' }]);
	});

	it('a save that lands after the user moved focus elsewhere leaves it there', async () => {
		let release!: () => void;
		net.gate = new Promise<void>((r) => (release = r));
		await startEditing();
		type(title(), 'Login two');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		const other = document.createElement('input');
		document.body.append(other);
		other.focus();
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsForm()).toBeNull();
		});
		await new Promise((r) => setTimeout(r, 20));
		expect(document.activeElement).toBe(other);
	});

	it('a failed save returns focus to the first field', async () => {
		net.patchAnswer = { status: 500, body: { outcome: 'error', code: 'internal', message: 'boom' } };
		await startEditing();
		type(title(), 'Login two');
		btn('Save details').focus();
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsAlert()).toContain('boom');
			expect(document.activeElement).toBe(title());
		});
	});

	it('an unchanged save returns focus to Edit details too', async () => {
		await startEditing();
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsForm()).toBeNull();
			expect(document.activeElement).toBe(btn('Edit details'));
		});
	});

	it('closes without a request when nothing changed', async () => {
		await startEditing();
		detailsForm().requestSubmit();
		await closed();
		expect(net.patches).toHaveLength(0);
	});

	it('refuses a blank title locally and keeps the draft', async () => {
		await startEditing();
		type(title(), '   ');
		detailsForm().requestSubmit();
		flushSync();
		expect(detailsAlert()).not.toBe('');
		expect(net.patches).toHaveLength(0);
		expect(title().value).toBe('   ');
	});

	it('keeps the draft through a live reload and saves on the followed version when another field moved', async () => {
		await startEditing();
		type(title(), 'Mine');
		net.epic = { ...net.epic, brief: 'changed elsewhere', version: 4 };
		await fireLive();
		flushSync();
		expect(title().value).toBe('Mine');
		expect(parent().value).toBe('ABC-1');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 4, title: 'Mine' });
	});

	it('does not follow the version when a details field changed meanwhile, and the save conflicts', async () => {
		await startEditing();
		type(title(), 'Mine');
		net.epic = { ...net.epic, initiative: 'ABC-2', version: 4 };
		await fireLive();
		flushSync();
		expect(parent().value).toBe('ABC-1'); // the draft is not overwritten by the reload
		net.patchAnswer = conflict;
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 3, title: 'Mine' });
	});

	it('after a conflict keeps the changed fields on the reloaded epic, then saves on the new version', async () => {
		await startEditing();
		type(title(), 'Mine');
		net.epic = { ...net.epic, initiative: 'ABC-2', version: 4 };
		net.patchAnswer = conflict;
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(notice()).toContain('Someone else changed');
		});
		expect(detailsForm()).not.toBeNull();
		expect(title().value).toBe('Mine');
		expect(parent().value).toBe('ABC-2');
		net.patchAnswer = null;
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(2));
		expect(net.patches[1]).toEqual({ version: 4, title: 'Mine' });
	});

	it('after a conflict keeps the changed initiative and takes the reloaded title', async () => {
		await startEditing();
		choose('ABC-2');
		net.epic = { ...net.epic, title: 'Theirs', version: 4 };
		net.patchAnswer = conflict;
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(notice()).toContain('Someone else changed');
		});
		expect(parent().value).toBe('ABC-2');
		expect(title().value).toBe('Theirs');
		net.patchAnswer = null;
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(2));
		expect(net.patches[1]).toEqual({ version: 4, initiative: 'ABC-2' });
	});

	it('after a conflict keeps a detach (null) and takes the reloaded title', async () => {
		await startEditing();
		choose('');
		net.epic = { ...net.epic, title: 'Theirs', version: 4 };
		net.patchAnswer = conflict;
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(notice()).toContain('Someone else changed');
		});
		expect(parent().value).toBe('');
		expect(title().value).toBe('Theirs');
		net.patchAnswer = null;
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(2));
		expect(net.patches[1]).toEqual({ version: 4, initiative: null });
	});

	it('Cancel leaves an unrelated page notice alone', async () => {
		await open();
		btn('Mark discovery complete').click();
		// the discovery POST is not stubbed: it fails with an unexpected request, which becomes the notice
		await vi.waitFor(() => {
			flushSync();
			expect(notice()).not.toBe('');
		});
		const before = notice();
		btn('Edit details').click();
		flushSync();
		btn('Cancel').click();
		flushSync();
		expect(notice()).toBe(before);
	});

	it('keeps the editor open with the draft and the error in the form when the server fails', async () => {
		await startEditing();
		type(title(), 'New');
		net.patchAnswer = { status: 500, body: { outcome: 'error', code: 'internal', message: 'boom' } };
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsAlert()).toContain('boom');
		});
		expect(notice()).toBe('');
		expect(title().value).toBe('New');
	});

	it('clears the error on Cancel and returns focus to Edit details', async () => {
		await startEditing();
		type(title(), 'New');
		net.patchAnswer = { status: 500, body: { outcome: 'error', code: 'internal', message: 'boom' } };
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsAlert()).toContain('boom');
		});
		btn('Cancel').click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(btn('Edit details'));
		});
		expect(document.body.textContent).not.toContain('boom');
		btn('Edit details').click();
		flushSync();
		expect(detailsAlert()).toBe('');
		expect(title().value).toBe('Login');
	});

	it('closes the editor when the page switches epic', async () => {
		await startEditing();
		expect(detailsForm()).not.toBeNull();
		route.fake!.params.set('key', 'ABC-6');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Elsewhere');
		});
		expect(detailsForm()).toBeNull();
	});

	it('names the story form, opened from New story in a dialog', async () => {
		await open();
		expect(document.body.querySelector('form[aria-label="New story"]')).toBeNull();
		btn('New story').click();
		flushSync();
		expect(document.body.querySelector('dialog form[aria-label="New story"]')).not.toBeNull();
	});
});

const storyItem = (key: string, delivery: Story['delivery'], reasons: Reason[] = [], kind: Story['kind'] = 'story') =>
	withReadiness({ key, title: `Story ${key}`, kind, epic: 'ABC-5', delivery, criteria: [{ text: 'works', done: false }] }, reasons);
const decisionOn = (id: number, on: string, title: string) => decision({ id, on, title, decider: 'a@b.se' });
const strip = () => document.body.querySelector('[role="group"][aria-label="Status"]')?.textContent ?? '';

describe('epic page: stories and status', () => {
	it('lists each story with a title link and unfolds its criteria from the chevron', async () => {
		net.stories = [storyItem('ABC-7', 'todo'), storyItem('ABC-8', 'in_progress')];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('a[href="/epics/ABC-5?story=ABC-7"]')?.textContent).toBe('Story ABC-7');
		});
		expect(document.title).toBe('Login · Reeve');
		const chevron = document.body.querySelector('button[aria-label="Criteria for ABC-7"]') as HTMLButtonElement;
		expect(chevron.getAttribute('aria-expanded')).toBe('false');
		expect(document.body.querySelector('input[aria-label="criterion 1"]')).toBeNull();
		chevron.click();
		flushSync();
		expect(chevron.getAttribute('aria-expanded')).toBe('true');
		const panel = document.getElementById(chevron.getAttribute('aria-controls')!)!;
		expect((panel.querySelector('input[aria-label="criterion 1"]') as HTMLInputElement).value).toBe('works');
		// Rows open independently: opening ABC-8 leaves ABC-7 open.
		const other = document.body.querySelector('button[aria-label="Criteria for ABC-8"]') as HTMLButtonElement;
		other.click();
		flushSync();
		expect(chevron.getAttribute('aria-expanded')).toBe('true');
		expect(other.getAttribute('aria-expanded')).toBe('true');
		chevron.click();
		flushSync();
		expect(chevron.getAttribute('aria-expanded')).toBe('false');
		expect(panel.hidden).toBe(true);
		expect(document.getElementById(other.getAttribute('aria-controls')!)!.hidden).toBe(false);
	});

	it('keeps an unsaved criteria draft through folding the row and opening it again', async () => {
		net.stories = [storyItem('ABC-7', 'todo')];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('button[aria-label="Criteria for ABC-7"]')).not.toBeNull();
		});
		const chevron = document.body.querySelector('button[aria-label="Criteria for ABC-7"]') as HTMLButtonElement;
		chevron.click();
		flushSync();
		const text = () => document.getElementById('criteria-ABC-7')!.querySelector('input[aria-label="criterion 1"]') as HTMLInputElement;
		text().value = 'works offline';
		text().dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		chevron.click();
		flushSync();
		chevron.click();
		flushSync();
		expect(text().value).toBe('works offline');
	});

	it('shows Bug and Chore on rows of those kinds, as board cards do', async () => {
		net.stories = [
			storyItem('ABC-7', 'todo', [], 'bug'),
			storyItem('ABC-8', 'todo', [], 'chore'),
			storyItem('ABC-9', 'todo')
		];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelectorAll('.stories > li')).toHaveLength(3);
		});
		const kinds = [...document.body.querySelectorAll('.stories > li')].map((li) => li.querySelector('.kind')?.textContent ?? null);
		expect(kinds).toEqual(['Bug', 'Chore', null]);
	});

	it('counts under the returned key when the URL names the epic by an alias', async () => {
		route.fake!.params.set('key', 'JIRA-55');
		net.stories = [storyItem('ABC-7', 'todo', [{ reason: 'no_criteria' }]), storyItem('ABC-8', 'done')];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(strip()).toContain('1 story needs criteria');
		});
		expect(strip()).toContain('1 of 2 stories done');
	});

	for (const how of ['Close', 'Escape'] as const) {
		it(`keeps the New story dialog open on ${how} while the create is pending`, async () => {
			await open();
			btn('New story').click();
			flushSync();
			const form = () => document.body.querySelector('dialog form[aria-label="New story"]') as HTMLFormElement | null;
			const input = form()!.querySelector('input') as HTMLInputElement;
			type(input, 'Pending story');
			let release!: () => void;
			net.storyGate = new Promise<void>((r) => (release = r));
			form()!.requestSubmit();
			await vi.waitFor(() => expect(net.storyPosts).toHaveLength(1));
			flushSync();
			const dialog = document.body.querySelector('dialog') as HTMLDialogElement;
			const close = dialog.querySelector('button[aria-label="Close"]') as HTMLButtonElement;
			expect(close.disabled).toBe(true);
			if (how === 'Close') close.click();
			else expect(dialog.dispatchEvent(new Event('cancel', { cancelable: true }))).toBe(false);
			flushSync();
			expect(form()).not.toBeNull();
			release();
			await vi.waitFor(() => {
				flushSync();
				expect(form()).toBeNull();
			});
		});
	}

	it('counts the todo stories the server holds back for criteria and the progress', async () => {
		net.stories = [storyItem('ABC-7', 'todo', [{ reason: 'no_criteria' }]), storyItem('ABC-8', 'testing', [{ reason: 'no_criteria' }]), storyItem('ABC-9', 'done')];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(strip()).toContain('1 story needs criteria');
		});
		expect(strip()).toContain('1 of 3 stories done');
		expect(document.body.querySelector('a[href="#stories"]')).not.toBeNull();
	});

	it('names blocking decisions on the epic and on its initiative, not those on other epics', async () => {
		net.decisions = [decisionOn(1, 'ABC-5', 'Auth provider')];
		net.initiativeDecisions = [decisionOn(2, 'ABC-1', 'Budget'), decisionOn(3, 'ABC-4', 'Elsewhere'), decisionOn(1, 'ABC-5', 'Auth provider')];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(strip()).toContain('2 blocking decisions');
		});
		expect(strip()).toContain('on Login, Orion');
		expect(net.initiativeReads).toContain('ABC-1');
	});

	it("rings a story row someone else just changed, like a board card", async () => {
		net.stories = [storyItem('ABC-7', 'todo'), storyItem('ABC-8', 'todo')];
		await open();
		liveState.recent = [
			{ entry: activity({ entity_type: 'story', entity_key: 'ABC-8', actor: 'erik@x.se', action: 'moved' }), arrived: Date.now() }
		];
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelectorAll('.stories > li.fresh')).toHaveLength(1);
		});
		const row = document.body.querySelector('.stories > li.fresh')!;
		expect(row.textContent).toContain('erik moved this');
		// The ring is app.css's live-change box-shadow: the row must match a selector of that rule, not only carry the class.
		const ring = [...appCss.matchAll(/([^{}]+)\{([^}]*)\}/g)]
			.filter(([, sel, body]) => sel.includes('.fresh') && body.includes('box-shadow'))
			.map(([, sel]) => sel.replace(/\/\*[\s\S]*?\*\//g, '').trim())
			.join(', ');
		expect(ring).not.toBe('');
		expect(row.matches(ring)).toBe(true);
		expect(document.body.querySelector('.stories > li:not(.fresh)')!.matches(ring)).toBe(false);
		liveState.recent = [];
	});

	it('unfolds accepted decisions into the same single list, blocking ones first', async () => {
		net.decisions = [decisionOn(1, 'ABC-5', 'Auth provider'), { ...decisionOn(2, 'ABC-5', 'Stack'), check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeDefined();
		});
		const section = document.getElementById('decisions')!;
		expect(section.textContent).toContain('Auth provider');
		expect(section.textContent).not.toContain('Stack');
		btn('1 accepted decision').click();
		flushSync();
		const fold = btn('1 accepted decision');
		expect(fold.getAttribute('aria-controls')).toBe('decisions-rest');
		const rest = document.getElementById('decisions-rest')!;
		expect(rest.textContent).toContain('Stack');
		expect(rest.textContent).not.toContain('Auth provider');
		const text = section.textContent!;
		expect(text.indexOf('Auth provider')).toBeLessThan(text.indexOf('Stack'));
	});

	it('reads no initiative decisions for an epic under no initiative', async () => {
		net.epic = { ...net.epic, initiative: null };
		await open();
		expect(net.initiativeReads).toEqual([]);
		expect(strip()).toContain('Nothing blocking');
	});
});

describe('epic page: New story across a page change', () => {
	const form = () => document.body.querySelector('dialog form[aria-label="New story"]') as HTMLFormElement | null;
	const closeBtn = () => document.body.querySelector('dialog button[aria-label="Close"]') as HTMLButtonElement;
	const titleInput = () => form()!.querySelector('input') as HTMLInputElement;
	async function startCreate(text: string) {
		btn('New story').click();
		flushSync();
		type(titleInput(), text);
		form()!.requestSubmit();
	}
	async function moveToOther() {
		route.fake!.params.set('key', 'ABC-6');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Elsewhere');
			expect(btn('New story')).toBeDefined();
		});
	}

	it('an older create failing after the page changed leaves the newer dialog busy and not closable', async () => {
		net.deferCreates = true;
		await open();
		await startCreate('First');
		await vi.waitFor(() => expect(net.createQueue).toHaveLength(1));
		await moveToOther();
		await startCreate('Second');
		await vi.waitFor(() => expect(net.createQueue).toHaveLength(2));
		flushSync();
		expect(closeBtn().disabled).toBe(true);
		net.createQueue[0](false); // the first page's create fails now
		await new Promise((r) => setTimeout(r, 20));
		flushSync();
		expect(closeBtn().disabled).toBe(true);
		expect(titleInput().disabled).toBe(true);
		expect(form()!.querySelector('[role="alert"]')).toBeNull();
		net.createQueue[1](true);
		await vi.waitFor(() => {
			flushSync();
			expect(form()).toBeNull();
		});
	});

	it('an older create succeeding after the page changed does not close the newer dialog', async () => {
		net.deferCreates = true;
		await open();
		await startCreate('First');
		await vi.waitFor(() => expect(net.createQueue).toHaveLength(1));
		await moveToOther();
		btn('New story').click();
		flushSync();
		type(titleInput(), 'Half typed');
		const reads = net.otherReads;
		net.createQueue[0](true);
		// The stale success has run its callback once the refresh it starts reads the page's epic.
		await vi.waitFor(() => expect(net.otherReads).toBeGreaterThan(reads));
		flushSync();
		expect(form()).not.toBeNull();
		expect(titleInput().value).toBe('Half typed');
		expect(closeBtn().disabled).toBe(false);
	});
});

describe('epic page: Add decision across a page change', () => {
	const form = () => document.body.querySelector('dialog form[aria-label="Add decision"]') as HTMLFormElement | null;
	const closeBtn = () => document.body.querySelector('dialog button[aria-label="Close"]') as HTMLButtonElement;
	const input = (name: string) =>
		[...form()!.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith(name))!.querySelector('input') as HTMLInputElement;
	function openDialog() {
		btn('+ Add decision').click();
		flushSync();
	}
	function submit(text: string) {
		openDialog();
		type(input('Title'), text);
		type(input('Decider'), 'dan@example.com');
		form()!.requestSubmit();
	}
	async function moveToOther() {
		route.fake!.params.set('key', 'ABC-6');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Elsewhere');
			expect(btn('+ Add decision')).toBeDefined();
		});
	}

	it('an older create failing after the page changed leaves the newer dialog busy and its alert empty', async () => {
		await open();
		submit('First');
		await vi.waitFor(() => expect(net.decisionQueue).toHaveLength(1));
		await moveToOther();
		submit('Second');
		await vi.waitFor(() => expect(net.decisionQueue).toHaveLength(2));
		flushSync();
		expect(closeBtn().disabled).toBe(true);
		net.decisionQueue[0](false); // the first page's create fails now
		// Its reason reaches the page, naming the item it was sent for, not the newer dialog.
		await vi.waitFor(() => {
			flushSync();
			expect(notice()).toContain('ABC-5');
			expect(notice()).toContain('boom');
		});
		expect(closeBtn().disabled).toBe(true);
		expect(input('Title').disabled).toBe(true);
		expect(input('Title').value).toBe('Second');
		expect(form()!.querySelector('[role="alert"]')).toBeNull();
		net.decisionQueue[1](true);
		await vi.waitFor(() => {
			flushSync();
			expect(form()).toBeNull();
		});
		expect(net.decisionPosts.map((p) => p.on)).toEqual(['ABC-5', 'ABC-6']);
	});

	it('an older create succeeding after the page changed does not close the newer dialog', async () => {
		await open();
		submit('First');
		await vi.waitFor(() => expect(net.decisionQueue).toHaveLength(1));
		await moveToOther();
		openDialog();
		type(input('Title'), 'Half typed');
		const reads = net.otherReads;
		net.decisionQueue[0](true);
		// The stale success is handled once the refresh it starts reads the page's epic.
		await vi.waitFor(() => expect(net.otherReads).toBeGreaterThan(reads));
		flushSync();
		expect(form()).not.toBeNull();
		expect(input('Title').value).toBe('Half typed');
		expect(input('Title').disabled).toBe(false);
		expect(closeBtn().disabled).toBe(false);
	});
});

describe('epic page: urgent mark', () => {
	const mark = (over: Record<string, unknown> = {}) => ({ at: '2026-11-01T08:00:00Z', by: 'dan@example.com', reason: 'Customer down', expires_at: '2099-01-01T00:00:00Z', active: true, done: false, ...over });
	const menu = async () => {
		(document.body.querySelector('button[aria-label="Actions for ABC-5"]') as HTMLButtonElement).click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[role="menuitem"]')).not.toBeNull();
		});
		return [...document.body.querySelectorAll('[role="menuitem"]')] as HTMLButtonElement[];
	};
	const item = (items: HTMLButtonElement[], name: string) => items.find((b) => b.textContent?.trim() === name);

	it('offers Mark urgent only while there is no mark, and Clear urgent mark with one', async () => {
		await open();
		expect((await menu()).map((b) => b.textContent?.trim())).toEqual(['Edit brief', 'Mark urgent…']);
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		net.epic = { ...net.epic, urgent_mark: mark(), urgent_sources: [{ key: 'ABC-5', item: 'epic' }] };
		await open();
		const names = (await menu()).map((b) => b.textContent?.trim());
		expect(names).toContain('Mark urgent…');
		expect(names).toContain('Clear urgent mark');
	});

	it('marks with the reason typed and shows the own line', async () => {
		await open();
		item(await menu(), 'Mark urgent…')!.click();
		flushSync();
		const dialog = document.body.querySelector('dialog')!;
		expect(dialog.querySelector('h2')!.textContent).toBe('Mark ABC-5 urgent');
		const input = dialog.querySelector('input') as HTMLInputElement;
		input.value = 'Launch blocked';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		(dialog.querySelector('form') as HTMLFormElement).requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
			expect(document.body.querySelector('[data-testid="urgent-line"]')!.textContent).toContain('marked by dan');
		});
		expect(net.urgentWrites).toEqual([{ method: 'POST', path: '/api/v1/epics/ABC-5/urgent', body: { reason: 'Launch blocked' } }]);
		expect(document.body.querySelector('[data-testid="urgent-line"]')!.textContent).toContain("'Launch blocked'");
	});

	it('clears the own mark from the menu', async () => {
		net.epic = { ...net.epic, urgent_mark: mark(), urgent_sources: [{ key: 'ABC-5', item: 'epic' }] };
		await open();
		item(await menu(), 'Clear urgent mark')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[data-testid="urgent-line"]')).toBeNull();
		});
		expect(net.urgentWrites).toEqual([{ method: 'DELETE', path: '/api/v1/epics/ABC-5/urgent', body: null }]);
	});

	it('shows a derived source as a link with its title', async () => {
		net.stories = [withReadiness({ key: 'ABC-9', title: 'Fix it', epic: 'ABC-5' })];
		net.epic = { ...net.epic, urgent_mark: null, urgent_sources: [{ key: 'ABC-9', item: 'story' }] };
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[data-testid="urgent-line"]')!.textContent).toContain('via story ABC-9 Fix it');
		});
		const a = document.body.querySelector('[data-testid="urgent-line"] a') as HTMLAnchorElement;
		expect(a.textContent).toBe('ABC-9');
		expect(a.getAttribute('href')).toContain('story=ABC-9');
		expect(document.body.querySelector('[data-testid="urgent-line"]')!.textContent).not.toContain('marked by');
	});

	it('shows an expired own mark with Re-mark (prefilled) and Clear', async () => {
		const expires = new Date(Date.now() - 3 * 86_400_000 - 3_600_000).toISOString();
		net.epic = { ...net.epic, urgent_mark: mark({ active: false, expires_at: expires }), urgent_sources: [] };
		await open();
		const line = document.body.querySelector('[data-testid="urgent-expired"]')!;
		expect(line.textContent).toContain('Urgent mark expired 3 d ago');
		(line.querySelector('button[aria-label="Re-mark ABC-5"]') as HTMLButtonElement).click();
		flushSync();
		expect((document.body.querySelector('dialog input') as HTMLInputElement).value).toBe('Customer down');
		(document.body.querySelector('dialog form') as HTMLFormElement).requestSubmit();
		await vi.waitFor(() => expect(net.urgentWrites).toHaveLength(1));
		expect(net.urgentWrites[0]).toMatchObject({ method: 'POST', body: { reason: 'Customer down' } });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[data-testid="urgent-expired"]')).toBeNull();
		});
	});

	it('clears an expired mark with its Clear button', async () => {
		net.epic = { ...net.epic, urgent_mark: mark({ active: false, expires_at: '2026-11-01T00:00:00Z' }), urgent_sources: [] };
		await open();
		(document.body.querySelector('button[aria-label="Clear urgent mark on ABC-5"]') as HTMLButtonElement).click();
		await vi.waitFor(() => expect(net.urgentWrites).toEqual([{ method: 'DELETE', path: '/api/v1/epics/ABC-5/urgent', body: null }]));
	});

	it('keeps the dialog open and shows a failed write inside it', async () => {
		await open();
		item(await menu(), 'Mark urgent…')!.click();
		flushSync();
		net.urgentFail = true;
		(document.body.querySelector('dialog form') as HTMLFormElement).requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog [role="alert"]')?.textContent).toContain('boom');
		});
	});
});
