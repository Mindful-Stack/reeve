// @vitest-environment jsdom
// The initiative page's details editor and epic creation: only changed fields are sent against the version the editor
// started from, a reload or live event never wipes the draft, a conflict keeps the user's fields on the reloaded item,
// Cancel clears the error and returns focus, and a created epic appears after the refetch.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ActivityEntry, Epic, Initiative, Reason, RequiredDecision, Story, StoryWithReadiness } from '#lib/api/types.ts';
import { activity, decision, epic, withReadiness } from '#lib/test/fixtures.ts';
import type { FakePage } from '#lib/test/net.ts';
import Page from './+page.svelte';

const live = vi.hoisted(() => ({ fns: [] as (() => unknown)[], polls: [] as (() => unknown)[] }));
/** A live event: every registered refetch (the page's and its children's) runs. */
const fireLive = () => Promise.all(live.fns.map((f) => f()));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => {
		live.fns.push(fn);
	},
	// The two-minute refetches, run by hand: every registered one (the page's and its lists').
	everyVisible: (fn: () => unknown) => {
		live.polls.push(fn);
	}
}));
const route = vi.hoisted(() => ({ fake: null as null | FakePage }));
// A reactive key, so a test can switch the page to another initiative.
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	route.fake = fakePage({ url: '/initiatives/ABC-1', params: { key: 'ABC-1' } });
	return { page: route.fake.page };
});

type Patched = { version: number; title?: string; target_month?: string | null; owner?: string | null };

const net = vi.hoisted(() => {
	const ini: Initiative = { key: 'ABC-1', title: 'Orion', brief: '', horizon: 'Now', version: 3, target_month: null, owner: null, urgent_mark: null, urgent_sources: [] };
	const s = {
		ini,
		epics: [] as Epic[],
		patches: [] as Patched[],
		urgentFail: false,
		urgentWrites: [] as { method: string; path: string; body: unknown }[],
		epicPosts: [] as { title: string; initiative: string | null; idempotency_key: string }[],
		patchAnswer: null as null | { status: number; body: unknown },
		gate: null as null | Promise<void>,
		decisions: [] as RequiredDecision[],
		decisionQueries: [] as string[],
		stories: [] as StoryWithReadiness[],
		activity: [] as ActivityEntry[],
		decisionPatches: [] as unknown[],
		decisionPosts: [] as unknown[],
		decisionAnswer: null as null | { status: number; body: unknown },
		/** While set, a decision create POST answers only once it resolves. */
		decisionGate: null as null | Promise<void>,
		/** While set, each decision create POST answers only when its queue entry is called: true succeeds, false fails. */
		deferDecisions: false,
		decisionQueue: [] as ((ok: boolean) => void)[],
		/** While set, each create POST answers only when its queue entry is called: true succeeds, false fails. */
		deferCreates: false,
		createQueue: [] as ((ok: boolean) => void)[],
		/** GETs of ABC-2, the initiative the New epic tests move to: a create's refresh reads it once the create answers. */
		otherReads: 0
	};
	return s;
});
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure, json } = await import('#lib/test/net.ts');
	const { decision } = await import('#lib/test/fixtures.ts');
	const s = net;
	installFetch(async ({ method, url, path, body }) => {
		if ((method === 'POST' || method === 'DELETE') && path === '/api/v1/initiatives/ABC-1/urgent') {
			s.urgentWrites.push({ method, path, body });
			if (s.urgentFail) return failure('boom');
			const mark = method === 'POST' ? { at: '2026-11-09T08:00:00Z', by: 'dan@example.com', reason: (body as { reason: string | null }).reason, expires_at: '2099-01-01T00:00:00Z', active: true, done: false } : null;
			s.ini = { ...s.ini, urgent_mark: mark, urgent_sources: mark ? [{ key: 'ABC-1', item: 'initiative' }] : [] };
			return s.ini;
		}
		if (method === 'GET' && path === '/api/v1/initiatives/ABC-1') return s.ini;
		// An external reference: the server resolves it and answers with the canonical key.
		if (method === 'GET' && path === '/api/v1/initiatives/JIRA-7') return s.ini;
		if (method === 'GET' && path === '/api/v1/initiatives/ABC-2') {
			s.otherReads++;
			return { ...s.ini, key: 'ABC-2', title: 'Other' };
		}
		if (method === 'GET' && path === '/api/v1/epics') return s.epics;
		if (method === 'GET' && path === '/api/v1/stories') return s.stories;
		if (method === 'GET' && path === '/api/v1/activity') return s.activity;
		if (method === 'GET' && path === '/api/v1/decisions') {
			s.decisionQueries.push(url.search);
			return s.decisions;
		}
		if (method === 'PATCH' && path.startsWith('/api/v1/decisions/')) {
			const b = body as Record<string, unknown>;
			s.decisionPatches.push(b);
			return null;
		}
		if (method === 'PATCH' && path === '/api/v1/initiatives/ABC-1') {
			const b = body as Patched;
			s.patches.push(b);
			if (s.gate) await s.gate;
			if (s.patchAnswer) return json(s.patchAnswer.body, s.patchAnswer.status);
			s.ini = { ...s.ini, ...b, version: s.ini.version + 1 };
			return s.ini;
		}
		if (method === 'POST' && path === '/api/v1/decisions') {
			const b = body as { title: string; decider: string; on: string };
			s.decisionPosts.push(b);
			if (s.decisionGate) await s.decisionGate;
			if (s.deferDecisions) {
				const ok = await new Promise<boolean>((r) => s.decisionQueue.push(r));
				if (!ok) return failure('boom');
			}
			if (s.decisionAnswer) return json(s.decisionAnswer.body, s.decisionAnswer.status);
			return decision({ id: 77, on: b.on, on_type: 'initiative', title: b.title, decider: b.decider });
		}
		if (method === 'POST' && path === '/api/v1/epics') {
			const b = body as { title: string; initiative: string | null; idempotency_key: string };
			s.epicPosts.push(b);
			if (s.deferCreates) {
				const ok = await new Promise<boolean>((r) => s.createQueue.push(r));
				if (!ok) return failure('boom');
			}
			const e: Epic = { key: 'ABC-9', title: b.title, version: 1, brief: '', initiative: b.initiative, urgent_mark: null, urgent_sources: [] };
			s.epics = [...s.epics, e];
			return e;
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
	live.polls.length = 0;
	document.body.innerHTML = '';
	net.ini = { key: 'ABC-1', title: 'Orion', brief: '', horizon: 'Now', version: 3, target_month: null, owner: null, urgent_mark: null, urgent_sources: [] };
	net.epics = [];
	net.patches.length = 0;
	net.urgentWrites.length = 0;
	net.urgentFail = false;
	net.epicPosts.length = 0;
	net.otherReads = 0;
	net.patchAnswer = null;
	net.gate = null;
	net.decisions = [];
	net.decisionQueries.length = 0;
	net.stories = [];
	net.activity = [];
	net.decisionPatches.length = 0;
	net.decisionPosts.length = 0;
	net.decisionAnswer = null;
	net.decisionGate = null;
	net.deferDecisions = false;
	net.decisionQueue.length = 0;
	net.deferCreates = false;
	net.createQueue.length = 0;
	route.fake!.reset();
});

const btn = (name: string) => [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === name) as HTMLButtonElement;
/** A decision row's Actions menu trigger (Edit and Close live in it) and its Discussion toggle, within `scope`. */
const decisionActions = (scope = '#decisions') => document.querySelector(`${scope} [data-field="actions"]`) as HTMLButtonElement | null;
const expandDiscussion = (scope: string) => {
	(document.querySelector(`${scope} [data-field="discussion"]`) as HTMLButtonElement).click();
	flushSync();
};
/** Opens the decision's editor the way a user does: the row's Actions menu, then Edit. */
const editDecision = (scope = '#decisions') => {
	decisionActions(scope)!.click();
	flushSync();
	([...document.body.querySelectorAll('[role="menuitem"]')].find((b) => b.textContent === 'Edit') as HTMLButtonElement).click();
	flushSync();
};
const detailsForm = () => document.body.querySelector('form[aria-label="Initiative details"]') as HTMLFormElement;
function field(label: string): HTMLInputElement {
	const l = [...detailsForm().querySelectorAll('label')].find((x) => x.textContent?.trim().startsWith(label));
	return l!.querySelector('input') as HTMLInputElement;
}
function type(el: HTMLInputElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
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
const detailsAlert = () => detailsForm()?.querySelector('[role="alert"]')?.textContent ?? '';
const notice = () => document.body.querySelector('.notice')?.textContent ?? '';

describe('initiative page: edit details', () => {
	it('sends only the changed field with the version the editor started from', async () => {
		await startEditing();
		expect(field('Title').value).toBe('Orion');
		type(field('Owner'), 'Me@Example.com ');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 3, owner: 'Me@Example.com' });
		await vi.waitFor(() => {
			flushSync();
			expect(detailsForm()).toBeNull();
		});
		expect(document.body.querySelector('.meta [title]')).not.toBeNull();
	});

	it('opening the editor focuses its first field', async () => {
		await startEditing();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(field('Title'));
		});
	});

	it('a successful save returns focus to Edit details', async () => {
		await startEditing();
		type(field('Title'), 'Orion two');
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsForm()).toBeNull();
			expect(document.activeElement).toBe(btn('Edit details'));
		});
		expect(net.patches).toEqual([{ version: 3, title: 'Orion two' }]);
	});

	it('a save that lands after the user moved focus elsewhere leaves it there', async () => {
		let release!: () => void;
		net.gate = new Promise<void>((r) => (release = r));
		await startEditing();
		type(field('Title'), 'Orion two');
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
		type(field('Title'), 'Orion two');
		btn('Save details').focus();
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsForm().querySelector('[role="alert"]')?.textContent).toContain('boom');
			expect(document.activeElement).toBe(field('Title'));
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
		await vi.waitFor(() => {
			flushSync();
			expect(detailsForm()).toBeNull();
		});
		expect(net.patches).toHaveLength(0);
	});

	it('refuses a malformed month locally and keeps the draft', async () => {
		await startEditing();
		type(field('Target month'), '2026-13');
		detailsForm().requestSubmit();
		flushSync();
		expect(detailsAlert()).toContain('YYYY-MM');
		expect(net.patches).toHaveLength(0);
		expect(field('Target month').value).toBe('2026-13');
	});

	it('keeps the draft through a live reload and saves on the followed version when another field moved', async () => {
		await startEditing();
		type(field('Owner'), 'a@example.com');
		net.ini = { ...net.ini, brief: 'changed elsewhere', version: 4 };
		await fireLive();
		flushSync();
		expect(field('Owner').value).toBe('a@example.com');
		expect(field('Title').value).toBe('Orion');
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0]).toEqual({ version: 4, owner: 'a@example.com' });
	});

	it('does not follow the version when a details field changed meanwhile, and the save conflicts', async () => {
		await startEditing();
		type(field('Owner'), 'a@example.com');
		net.ini = { ...net.ini, title: 'Renamed', version: 4 };
		await fireLive();
		flushSync();
		expect(field('Title').value).toBe('Orion'); // the draft is not overwritten by the reload
		net.patchAnswer = { status: 409, body: { outcome: 'error', code: 'conflict', message: 'x', current: 4 } };
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(1));
		expect(net.patches[0].version).toBe(3);
	});

	it('after a conflict keeps the changed fields on the reloaded item, then saves on the new version', async () => {
		await startEditing();
		type(field('Owner'), 'a@example.com');
		net.ini = { ...net.ini, title: 'Renamed', version: 4 };
		net.patchAnswer = { status: 409, body: { outcome: 'error', code: 'conflict', message: 'x', current: 4 } };
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(notice()).toContain('Someone else changed');
		});
		expect(detailsForm()).not.toBeNull();
		expect(field('Owner').value).toBe('a@example.com');
		expect(field('Title').value).toBe('Renamed');
		net.patchAnswer = null;
		detailsForm().requestSubmit();
		await vi.waitFor(() => expect(net.patches).toHaveLength(2));
		expect(net.patches[1]).toEqual({ version: 4, owner: 'a@example.com' });
	});

	it('keeps the editor open with the draft when the server refuses', async () => {
		await startEditing();
		type(field('Title'), 'New');
		net.patchAnswer = { status: 500, body: { outcome: 'error', code: 'internal', message: 'boom' } };
		detailsForm().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(detailsAlert()).toContain('boom');
		});
		expect(notice()).toBe('');
		expect(field('Title').value).toBe('New');
	});

	it('clears the error on Cancel and returns focus to Edit details', async () => {
		await startEditing();
		type(field('Title'), 'New');
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
		expect(field('Title').value).toBe('Orion');
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

	it('closes the details editor and the new-epic dialog, emptying its form, when the page switches initiative', async () => {
		await startEditing();
		btn('New epic').click();
		flushSync();
		const epicTitle = () =>
			[...document.body.querySelectorAll('form[aria-label="New epic"] label')]
				.find((l) => l.textContent?.startsWith('Title'))!
				.querySelector('input') as HTMLInputElement;
		epicTitle().value = 'Half typed';
		epicTitle().dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		expect(detailsForm()).not.toBeNull();
		route.fake!.params.set('key', 'ABC-2');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Other');
		});
		expect(detailsForm()).toBeNull();
		expect(document.body.querySelector('dialog')).toBeNull();
		btn('New epic').click();
		flushSync();
		expect(epicTitle().value).toBe('');
	});
});

describe('initiative page: new epic', () => {
	it('creates an epic under this initiative from the dialog, closes it and lists the epic after the refetch', async () => {
		await open();
		expect(document.body.querySelector('form[aria-label="New epic"]')).toBeNull();
		btn('New epic').click();
		flushSync();
		const f = document.body.querySelector('dialog form[aria-label="New epic"]') as HTMLFormElement;
		const title = [...f.querySelectorAll('label')].find((l) => l.textContent?.startsWith('Title'))!.querySelector('input')!;
		title.value = 'Fresh epic';
		title.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		f.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('a[href="/epics/ABC-9"]')?.textContent).toBe('Fresh epic');
		});
		expect(net.epicPosts).toHaveLength(1);
		expect(net.epicPosts[0]).toEqual({ idempotency_key: expect.any(String), title: 'Fresh epic', brief: null, initiative: 'ABC-1' });
		expect(document.body.querySelector('dialog')).toBeNull();
	});
});

describe('initiative decisions', () => {
	it('lists every decision under the initiative, each linked where it lives', async () => {
		net.epics = [epic({ key: 'ABC-5', title: 'Hosting', initiative: 'ABC-1' })];
		net.decisions = [
			decision({ id: 4, on: 'ABC-5', on_type: 'epic', title: 'Pick a host', decider: 'dan@example.com', created_at: '2026-10-31T09:00:00Z' })
		];
		app = mount(Page, { target: document.body });
		const link = await vi.waitFor(() => {
			flushSync();
			const a = document.body.querySelector('#decisions a.key');
			expect(a).not.toBeNull();
			return a!;
		});
		expect(net.decisionQueries).toContain('?initiative=ABC-1');
		expect(link.getAttribute('href')).toBe('/epics/ABC-5');
		expect(link.textContent).toContain('ABC-5');
		expect(link.textContent).toContain('■');
	});
});

describe('initiative page: status strip, epic cards and decisions', () => {
	const dec = (id: number, on: string, title: string, check: RequiredDecision['check'] = { kind: 'unchecked' }) =>
		decision({ id, on, on_type: 'epic', title, decider: 'dan@example.com', check, created_at: '2026-10-31T09:00:00Z' });
	const st = (key: string, epic: string, delivery: Story['delivery'], reasons: Reason[] = []) =>
		withReadiness({ key, epic, delivery, title: key }, reasons);

	it('counts each epic from its stories and names what it waits on', async () => {
		net.epics = [
			epic({ key: 'ABC-5', title: 'Hosting', initiative: 'ABC-1' }),
			epic({ key: 'ABC-6', title: 'Email', initiative: 'ABC-1' })
		];
		net.stories = [
			st('ABC-10', 'ABC-5', 'todo'),
			st('ABC-11', 'ABC-5', 'in_progress'),
			st('ABC-12', 'ABC-5', 'done'),
			st('ABC-13', 'ABC-6', 'todo', [{ reason: 'no_criteria' }]),
			st('ABC-99', 'ABC-77', 'done') // another initiative's epic
		];
		net.decisions = [dec(4, 'ABC-6', 'Email provider'), dec(5, 'ABC-5', 'Settled', { kind: 'accepted', commit: 'abcdef0123' })];
		await open();
		const card = (title: string) => [...document.body.querySelectorAll('#epics li')].find((l) => l.textContent?.includes(title))!;
		await vi.waitFor(() => {
			flushSync();
			expect(card('Hosting').textContent).toContain('3 stories');
		});
		expect(card('Hosting').textContent).toContain('1 ready · 1 in progress');
		expect(card('Hosting').textContent).toContain('1 of 3 done');
		expect(card('Email').textContent).toContain('Waiting on Email provider');
		const strip = document.body.querySelector('[role="group"][aria-label="Status"]')!;
		expect(strip.textContent).toContain('1 blocking decision');
		expect(strip.textContent).toContain('on Email');
		expect(strip.textContent).toContain('1 of 4 stories done');
		expect(strip.querySelector('a')?.getAttribute('href')).toBe('#decisions');
		expect(document.getElementById('decisions')).not.toBeNull();
		expect(document.getElementById('epics')).not.toBeNull();
	});

	it('names a decision on the initiative on its epic cards when the URL names it by an external reference', async () => {
		route.fake!.params.set('key', 'JIRA-7');
		net.epics = [epic({ key: 'ABC-6', title: 'Email', initiative: 'ABC-1' })];
		net.decisions = [decision({ id: 4, on: 'ABC-1', on_type: 'initiative', title: 'Hosting region', decider: 'dan@example.com' })];
		await open();
		const card = () => [...document.body.querySelectorAll('#epics li')].find((l) => l.textContent?.includes('Email'));
		await vi.waitFor(() => {
			flushSync();
			expect(card()?.textContent).toContain('Waiting on Hosting region');
		});
	});

	it('links stories needing criteria to the epics', async () => {
		net.epics = [epic({ key: 'ABC-6', title: 'Email', initiative: 'ABC-1' })];
		net.stories = [st('ABC-13', 'ABC-6', 'todo', [{ reason: 'no_criteria' }])];
		await open();
		await vi.waitFor(() => {
			flushSync();
			const a = document.body.querySelector('[aria-label="Status"] a');
			expect(a?.textContent).toContain('1 story needs criteria');
			expect(a?.getAttribute('href')).toBe('#epics');
		});
	});

	it('folds the accepted decisions behind a disclosure', async () => {
		net.epics = [epic({ key: 'ABC-5', title: 'Hosting', initiative: 'ABC-1' })];
		net.decisions = [dec(4, 'ABC-5', 'Open one'), dec(5, 'ABC-5', 'Settled', { kind: 'accepted', commit: 'abcdef0123' })];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.getElementById('decisions')!.textContent).toContain('Open one');
		});
		const fold = btn('1 accepted decision');
		expect(fold.getAttribute('aria-expanded')).toBe('false');
		expect(document.getElementById('decisions')!.textContent).not.toContain('Settled');
		fold.click();
		flushSync();
		expect(fold.getAttribute('aria-expanded')).toBe('true');
		expect(document.getElementById('decisions-rest')!.textContent).toContain('Settled');
	});

	it('switches the discussion between comments and activity', async () => {
		await open();
		const tab = (name: string) => [...document.body.querySelectorAll('[role="tab"]')].find((t) => t.textContent === name) as HTMLButtonElement;
		const panel = () => document.getElementById('initiative-discussion-panel')!;
		expect(panel().getAttribute('aria-labelledby')).toBe(tab('Comments').id);
		tab('Activity').click();
		flushSync();
		expect(panel().getAttribute('aria-labelledby')).toBe(tab('Activity').id);
		expect(panel().textContent).toContain('No activity.');
	});
});

describe('initiative page: add decision and the fold', () => {
	const dialogForm = () => document.body.querySelector('dialog form[aria-label="Add decision"]') as HTMLFormElement | null;
	const input = (label: string) =>
		[...dialogForm()!.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith(label))!.querySelector('input') as HTMLInputElement;

	it('adds a decision from a dialog that closes on success', async () => {
		await open();
		expect(dialogForm()).toBeNull();
		btn('+ Add decision').click();
		flushSync();
		type(input('Title'), 'Pick a host');
		type(input('Decider'), 'dan@example.com');
		dialogForm()!.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(dialogForm()).toBeNull();
		});
		expect(net.decisionPosts).toEqual([
			{ idempotency_key: expect.any(String), on: 'ABC-1', title: 'Pick a host', decider: 'dan@example.com', due: null }
		]);
	});

	it('keeps the dialog and the draft open with the reason when the create fails', async () => {
		net.decisionAnswer = { status: 500, body: { outcome: 'error', code: 'internal', message: 'boom' } };
		await open();
		btn('+ Add decision').click();
		flushSync();
		type(input('Title'), 'Pick a host');
		type(input('Decider'), 'dan@example.com');
		dialogForm()!.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(dialogForm()!.querySelector('[role="alert"]')?.textContent).toContain('boom');
		});
		expect(input('Title').value).toBe('Pick a host');
		expect(notice()).toBe('');
	});

	for (const how of ['Close', 'Escape'] as const) {
		it(`keeps the dialog open on ${how} while the create is pending`, async () => {
			await open();
			btn('+ Add decision').click();
			flushSync();
			type(input('Title'), 'Pick a host');
			type(input('Decider'), 'dan@example.com');
			let release!: () => void;
			net.decisionGate = new Promise<void>((r) => (release = r));
			dialogForm()!.requestSubmit();
			await vi.waitFor(() => expect(net.decisionPosts).toHaveLength(1));
			flushSync();
			const dialog = document.body.querySelector('dialog') as HTMLDialogElement;
			const close = dialog.querySelector('button[aria-label="Close"]') as HTMLButtonElement;
			expect(close.disabled).toBe(true);
			if (how === 'Close') close.click();
			else expect(dialog.dispatchEvent(new Event('cancel', { cancelable: true }))).toBe(false);
			flushSync();
			expect(dialogForm()).not.toBeNull();
			release();
			await vi.waitFor(() => {
				flushSync();
				expect(dialogForm()).toBeNull();
			});
		});
	}

	it('folds only closed decisions behind "Show closed (N)"', async () => {
		net.decisions = [
			decision({ id: 4, on: 'ABC-1', on_type: 'initiative', title: 'Old one', decider: 'dan@example.com', created_at: '2026-10-31T09:00:00Z',
				closed: { by: 'dan@example.com', at: '2026-10-01T00:00:00Z', reason: 'moot' } })
		];
		await open();
		const fold = btn('Show closed (1)');
		expect(fold).toBeDefined();
		fold.click();
		flushSync();
		expect(document.getElementById('decisions-rest')!.textContent).toContain('Old one');
	});

	it('has no fold when there is nothing to fold, and Edit brief only in the menu', async () => {
		await open();
		expect([...document.body.querySelectorAll('#decisions button')].map((b) => b.textContent?.trim())).toEqual(['+ Add decision']);
		expect(btn('Edit brief')).toBeUndefined();
		(document.body.querySelector('button[aria-label="Actions for ABC-1"]') as HTMLButtonElement).click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[role="menuitem"]')?.textContent).toBe('Edit brief');
		});
	});
});

describe('initiative page: dialogs, fold and discussion state', () => {
	const cancel = () => document.body.querySelector('dialog')!.dispatchEvent(new Event('cancel', { cancelable: true }));
	const decisionInput = (label: string) =>
		[...document.body.querySelectorAll('dialog form[aria-label="Add decision"] label')]
			.find((l) => l.textContent?.trim().startsWith(label))!
			.querySelector('input') as HTMLInputElement;

	it('returns focus to New epic when its dialog closes', async () => {
		await open();
		btn('New epic').focus();
		btn('New epic').click();
		flushSync();
		expect(document.body.querySelector('dialog')).not.toBeNull();
		cancel();
		flushSync();
		expect(document.body.querySelector('dialog')).toBeNull();
		expect(document.activeElement).toBe(btn('New epic'));
	});

	it('Escape on Add decision keeps the draft and drops the error', async () => {
		net.decisionAnswer = { status: 500, body: { outcome: 'error', code: 'internal', message: 'boom' } };
		await open();
		btn('+ Add decision').click();
		flushSync();
		type(decisionInput('Title'), 'Pick a host');
		type(decisionInput('Decider'), 'dan@example.com');
		btn('Require decision').focus();
		(document.body.querySelector('dialog form') as HTMLFormElement).requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog [role="alert"]')?.textContent).toContain('boom');
			expect(document.activeElement).toBe(decisionInput('Title')); // focus back on the first field
		});
		cancel();
		flushSync();
		expect(document.body.querySelector('dialog')).toBeNull();
		btn('+ Add decision').click();
		flushSync();
		expect(decisionInput('Title').value).toBe('Pick a host');
		expect(document.body.querySelector('dialog [role="alert"]')).toBeNull();
	});

	it('a closed-only fold that gains an accepted decision hides the closed ones again', async () => {
		const base = decision({ on: 'ABC-1', on_type: 'initiative', decider: 'dan@example.com', due: null, record: null, pr_url: null,
			checked_at: null, created_at: '2026-10-31T09:00:00Z', version: 1 });
		const closed: RequiredDecision = { ...base, id: 4, title: 'Old one', check: { kind: 'unchecked' },
			closed: { by: 'dan@example.com', at: '2026-10-01T00:00:00Z', reason: 'moot' } };
		net.decisions = [closed];
		await open();
		btn('Show closed (1)').click();
		flushSync();
		expect(document.getElementById('decisions-rest')!.textContent).toContain('Old one');
		net.decisions = [closed, { ...base, id: 5, title: 'Settled', check: { kind: 'accepted', commit: 'abcdef0123' }, closed: null }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeDefined();
		});
		const rest = document.getElementById('decisions-rest')!.textContent;
		expect(rest).toContain('Settled');
		expect(rest).not.toContain('Old one');
	});

	it('keeps a half-written comment through a switch to Activity and back', async () => {
		await open();
		const box = () => document.body.querySelector('textarea[aria-label="comment on ABC-1"]') as HTMLTextAreaElement;
		box().value = 'half a thought';
		box().dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		const tab = (name: string) => [...document.body.querySelectorAll('[role="tab"]')].find((t) => t.textContent === name) as HTMLButtonElement;
		tab('Activity').click();
		flushSync();
		expect(box().closest('[hidden]')).not.toBeNull();
		tab('Comments').click();
		flushSync();
		expect(box().closest('[hidden]')).toBeNull();
		expect(box().value).toBe('half a thought');
	});
});

describe('initiative page: Codex review fixes', () => {
	const base = decision({ on: 'ABC-1', on_type: 'initiative', decider: 'dan@example.com', due: null, pr_url: null, checked_at: null,
		closed: null, created_at: '2026-10-31T09:00:00Z', version: 1 });
	const record: RequiredDecision['record'] = { repo: 'lore', path: 'adr/0009.md', record_id: 'ADR-0009' };

	it('shows the owner\'s title once, though the owner has activity', async () => {
		net.ini = { ...net.ini, owner: 'own@example.com' };
		net.activity = [activity({ id: 1, actor: 'own@example.com', action: 'edited', at: '2026-10-31T09:00:00Z',
			entity_type: 'initiative', entity_key: 'ABC-1', payload: null })];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelectorAll('[title="own@example.com"]')).toHaveLength(1);
		});
		const tab = [...document.body.querySelectorAll('[role="tab"]')].find((t) => t.textContent === 'Activity') as HTMLButtonElement;
		tab.click();
		flushSync();
		expect(document.getElementById('initiative-discussion-panel')!.textContent).toContain('edited');
	});

	it('keeps an unsaved decision edit, its focus and the open row when the decision moves from accepted to blocking', async () => {
		net.decisions = [{ ...base, id: 5, title: 'Settled', record, check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeDefined();
		});
		btn('1 accepted decision').click();
		flushSync();
		expandDiscussion('#decisions-rest');
		editDecision('#decisions-rest');
		const form = () => document.body.querySelector('form[aria-label="Edit decision Settled"]') as HTMLFormElement | null;
		const title = () =>
			[...form()!.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith('Title'))!.querySelector('input') as HTMLInputElement;
		type(title(), 'Settled, half renamed');
		title().focus();
		title().setSelectionRange(3, 7);
		// The record is superseded elsewhere: the decision blocks again and moves to the first group.
		net.decisions = [{ ...base, id: 5, title: 'Settled', record, check: { kind: 'not_accepted', commit: 'abcdef0123', status: 'superseded', superseded_by: 'ADR-0010' } }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeUndefined();
		});
		expect(document.getElementById('decisions-rest')?.textContent ?? '').not.toContain('Settled');
		expect(form()).not.toBeNull();
		expect(title().value).toBe('Settled, half renamed');
		// The row was re-created in the other group: it stays open and the field being edited keeps focus and selection.
		expect(form()!.closest('li')!.querySelector('[data-field="discussion"]')!.getAttribute('aria-expanded')).toBe('true');
		expect(document.activeElement).toBe(title());
		expect([title().selectionStart, title().selectionEnd]).toEqual([3, 7]);
	});

	it('refreshes the decisions once per two-minute tick, accepted ones included', async () => {
		net.decisions = [{ ...base, id: 5, title: 'Settled', record, check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await open();
		btn('1 accepted decision').click();
		// The unfolded accepted decision is on screen before the polls are counted.
		await vi.waitFor(() => {
			flushSync();
			expect(document.getElementById('decisions-rest')?.textContent).toContain('Settled');
		});
		const before = net.decisionQueries.length;
		await Promise.all(live.polls.map((f) => f()));
		await vi.waitFor(() => expect(net.decisionQueries.length).toBe(before + 1));
		await new Promise((r) => setTimeout(r, 20));
		expect(net.decisionQueries.length).toBe(before + 1);
	});
});

describe('initiative page: decision focus across groups', () => {
	it('does not take focus from another control when a decision changes group', async () => {
		const base = decision({ on: 'ABC-1', on_type: 'initiative', decider: 'dan@example.com', due: null, pr_url: null, checked_at: null,
			closed: null, created_at: '2026-10-31T09:00:00Z', version: 1, title: 'Settled' });
		const record: RequiredDecision['record'] = { repo: 'lore', path: 'adr/0009.md', record_id: 'ADR-0009' };
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await open();
		btn('1 accepted decision').click();
		flushSync();
		expandDiscussion('#decisions-rest');
		editDecision('#decisions-rest');
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement?.getAttribute('name')).toBe('title'); // Edit has put focus in the form
		});
		const other = btn('+ Add decision');
		other.focus(); // the user moved on before the update arrived
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'not_accepted', commit: 'abcdef0123', status: 'superseded', superseded_by: 'ADR-0010' } }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeUndefined();
		});
		expect(document.activeElement).toBe(other);
		// Later, focus falls to <body> and another reload arrives: the old field is not pulled back into focus.
		other.blur();
		expect(document.activeElement).toBe(document.body);
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeDefined();
		});
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'not_accepted', commit: 'abcdef0123', status: 'superseded', superseded_by: 'ADR-0010' } }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeUndefined();
		});
		expect(document.activeElement).toBe(document.body);
	});
});

describe('initiative page: decision rows keep their control through changes', () => {
	const base = decision({ on: 'ABC-1', on_type: 'initiative', decider: 'dan@example.com', due: null, pr_url: null, checked_at: null,
		closed: null, created_at: '2026-10-31T09:00:00Z', version: 1, title: 'Settled' });
	const record: RequiredDecision['record'] = { repo: 'lore', path: 'adr/0009.md', record_id: 'ADR-0009' };
	it('restores focus to the same Actions button though a PR link appeared and the title changed', async () => {
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await open();
		btn('1 accepted decision').click();
		flushSync();
		expandDiscussion('#decisions-rest');
		decisionActions('#decisions-rest')!.focus();
		expect(document.activeElement).toBe(decisionActions('#decisions-rest'));
		// Concurrently: the record went back to a proposal with an open PR (a new link in the row), and it was renamed.
		net.decisions = [{ ...base, id: 5, title: 'Renamed', record, pr_url: 'https://github.com/x/y/pull/1', check: { kind: 'proposed_pr_open' } }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(document.querySelector('#decisions-rest li')).toBeNull();
			expect(document.querySelector('#decisions li a[data-field="pr"]')).not.toBeNull();
		});
		expect(document.activeElement).toBe(decisionActions());
		expect((document.activeElement as HTMLElement).dataset.field).toBe('actions');
		expect(document.querySelector('#decisions li [data-field="discussion"]')!.getAttribute('aria-expanded')).toBe('true');
	});

	it('keeps a row pinned by focus in view while its Actions menu opens', async () => {
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'unchecked' } }];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(decisionActions()).not.toBeNull();
		});
		decisionActions()!.focus();
		// A refetch accepts it: the focused row stays pinned in the visible group under the closed fold.
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeDefined();
		});
		expect(decisionActions()).not.toBeNull();
		decisionActions()!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement?.getAttribute('role')).toBe('menuitem');
		});
		await new Promise((r) => setTimeout(r, 20));
		flushSync();
		expect(document.querySelector('#decisions li')).not.toBeNull();
		expect(document.querySelector('#decisions [role="menu"]')).not.toBeNull();
		expect(document.activeElement).not.toBe(document.body);
		// Focus leaving the menu for another control (Tab out) releases the pin: the row goes into the fold.
		btn('+ Add decision').focus();
		await vi.waitFor(() => {
			flushSync();
			expect(document.querySelector('#decisions li')).toBeNull();
		});
	});

	it('keeps a blocking decision being edited in view when it becomes accepted under a closed fold', async () => {
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'unchecked' } }];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.querySelector('#decisions li [data-field="discussion"]')).not.toBeNull();
		});
		expandDiscussion('#decisions li');
		editDecision();
		const title = () => document.querySelector('#decisions input[data-field="title"]') as HTMLInputElement;
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(title());
		});
		type(title(), 'Settled, half renamed');
		title().setSelectionRange(2, 5);
		net.decisions = [{ ...base, id: 5, record, check: { kind: 'accepted', commit: 'abcdef0123' } }];
		await fireLive();
		await vi.waitFor(() => {
			flushSync();
			expect(btn('1 accepted decision')).toBeDefined();
		});
		expect(btn('1 accepted decision').getAttribute('aria-expanded')).toBe('false');
		expect(title()).not.toBeNull(); // pinned in the visible group while it is edited
		expect(title().value).toBe('Settled, half renamed');
		expect(document.activeElement).toBe(title());
		expect([title().selectionStart, title().selectionEnd]).toEqual([2, 5]);

		// Closing the pinned editor hands focus to the row's Actions button; the row stays in view meanwhile.
		const editBtn = () => decisionActions('#decisions li');
		const form = () => document.querySelector('#decisions form.edit') as HTMLFormElement;
		const focusesEdit = async () => {
			await vi.waitFor(() => {
				flushSync();
				expect(editBtn()).not.toBeNull();
				expect(document.activeElement).toBe(editBtn());
			});
			await new Promise((r) => setTimeout(r, 20));
			flushSync();
			expect(document.activeElement).toBe(editBtn()); // still there: the pin held through the handoff
		};
		const reopen = async () => {
			editDecision();
			await vi.waitFor(() => {
				flushSync();
				expect(document.activeElement).toBe(title());
			});
		};

		(document.querySelector('#decisions [data-field="cancel"]') as HTMLButtonElement).click();
		await focusesEdit();

		await reopen();
		form().requestSubmit(); // unchanged Save
		await focusesEdit();
		expect(net.decisionPatches).toHaveLength(0);

		await reopen();
		type(title(), 'Settled for good');
		form().requestSubmit(); // successful Save
		await vi.waitFor(() => expect(net.decisionPatches).toHaveLength(1));
		await focusesEdit();

		// Focus leaves the row: the pin releases and the accepted decision goes into the closed fold.
		editBtn()!.blur();
		await vi.waitFor(() => {
			flushSync();
			expect(document.querySelector('#decisions li')).toBeNull();
		});
		expect(document.activeElement).toBe(document.body);
		expect(btn('1 accepted decision').getAttribute('aria-expanded')).toBe('false');
	});
});

describe('initiative page: New epic across a page change', () => {
	const form = () => document.body.querySelector('dialog form[aria-label="New epic"]') as HTMLFormElement | null;
	const closeBtn = () => document.body.querySelector('dialog button[aria-label="Close"]') as HTMLButtonElement;
	const titleInput = () => form()!.querySelector('input') as HTMLInputElement;
	async function startCreate(text: string) {
		btn('New epic').click();
		flushSync();
		type(titleInput(), text);
		form()!.requestSubmit();
	}
	async function moveToOther() {
		route.fake!.params.set('key', 'ABC-2');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Other');
			expect(btn('New epic')).toBeDefined();
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
		btn('New epic').click();
		flushSync();
		type(titleInput(), 'Half typed');
		const reads = net.otherReads;
		net.createQueue[0](true);
		// The stale success has run its callback once the refresh it starts reads the page's initiative.
		await vi.waitFor(() => expect(net.otherReads).toBeGreaterThan(reads));
		flushSync();
		expect(form()).not.toBeNull();
		expect(titleInput().value).toBe('Half typed');
		expect(closeBtn().disabled).toBe(false);
	});
});

describe('initiative page: Add decision across a page change', () => {
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
		route.fake!.params.set('key', 'ABC-2');
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Other');
			expect(btn('+ Add decision')).toBeDefined();
		});
	}

	it('an older create failing after the page changed leaves the newer dialog busy and its alert empty', async () => {
		net.deferDecisions = true;
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
			expect(notice()).toContain('ABC-1');
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
		expect((net.decisionPosts as { on: string }[]).map((p) => p.on)).toEqual(['ABC-1', 'ABC-2']);
	});

	it('an older create succeeding after the page changed does not close the newer dialog', async () => {
		net.deferDecisions = true;
		await open();
		submit('First');
		await vi.waitFor(() => expect(net.decisionQueue).toHaveLength(1));
		await moveToOther();
		openDialog();
		type(input('Title'), 'Half typed');
		const reads = net.otherReads;
		net.decisionQueue[0](true);
		// The stale success is handled once the refresh it starts reads the page's initiative.
		await vi.waitFor(() => expect(net.otherReads).toBeGreaterThan(reads));
		flushSync();
		expect(form()).not.toBeNull();
		expect(input('Title').value).toBe('Half typed');
		expect(input('Title').disabled).toBe(false);
		expect(closeBtn().disabled).toBe(false);
	});
});

describe('initiative page: urgent mark', () => {
	const mark = (over: Record<string, unknown> = {}) => ({ at: '2026-11-01T08:00:00Z', by: 'dan@example.com', reason: 'Customer down', expires_at: '2099-01-01T00:00:00Z', active: true, done: false, ...over });
	const menu = async () => {
		(document.body.querySelector('button[aria-label="Actions for ABC-1"]') as HTMLButtonElement).click();
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
		net.ini = { ...net.ini, urgent_mark: mark(), urgent_sources: [{ key: 'ABC-1', item: 'initiative' }] };
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
		expect(dialog.querySelector('h2')!.textContent).toBe('Mark ABC-1 urgent');
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
		expect(net.urgentWrites).toEqual([{ method: 'POST', path: '/api/v1/initiatives/ABC-1/urgent', body: { reason: 'Launch blocked' } }]);
		expect(document.body.querySelector('[data-testid="urgent-line"]')!.textContent).toContain("'Launch blocked'");
	});

	it('clears the own mark from the menu', async () => {
		net.ini = { ...net.ini, urgent_mark: mark(), urgent_sources: [{ key: 'ABC-1', item: 'initiative' }] };
		await open();
		item(await menu(), 'Clear urgent mark')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[data-testid="urgent-line"]')).toBeNull();
		});
		expect(net.urgentWrites).toEqual([{ method: 'DELETE', path: '/api/v1/initiatives/ABC-1/urgent', body: null }]);
	});

	it('shows a derived source as a link with its title', async () => {
		net.epics = [{ key: 'ABC-5', title: 'Login', brief: '', version: 1, initiative: 'ABC-1', urgent_mark: null, urgent_sources: [] } as Epic];
		net.stories = [withReadiness({ key: 'ABC-9', title: 'Fix it', epic: 'ABC-5' })];
		net.ini = { ...net.ini, urgent_mark: null, urgent_sources: [{ key: 'ABC-9', item: 'story' }] };
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
		net.ini = { ...net.ini, urgent_mark: mark({ active: false, expires_at: expires }), urgent_sources: [] };
		await open();
		const line = document.body.querySelector('[data-testid="urgent-expired"]')!;
		expect(line.textContent).toContain('Urgent mark expired 3 d ago');
		(line.querySelector('button[aria-label="Re-mark ABC-1"]') as HTMLButtonElement).click();
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
		net.ini = { ...net.ini, urgent_mark: mark({ active: false, expires_at: '2026-11-01T00:00:00Z' }), urgent_sources: [] };
		await open();
		(document.body.querySelector('button[aria-label="Clear urgent mark on ABC-1"]') as HTMLButtonElement).click();
		await vi.waitFor(() => expect(net.urgentWrites).toEqual([{ method: 'DELETE', path: '/api/v1/initiatives/ABC-1/urgent', body: null }]));
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

	it('puts the Urgent chip on an epic card that is urgent', async () => {
		net.epics = [
			{ key: 'ABC-5', title: 'Login', brief: '', version: 1, initiative: 'ABC-1', urgent_mark: null, urgent_sources: [{ key: 'ABC-9', item: 'story' }] } as Epic,
			{ key: 'ABC-6', title: 'Quiet', brief: '', version: 1, initiative: 'ABC-1', urgent_mark: null, urgent_sources: [] } as Epic
		];
		await open();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Quiet');
		});
		const cards = [...document.body.querySelectorAll('li.epic-card')];
		expect(cards.find((c) => c.textContent?.includes('Login'))!.querySelector('.urgent-chip')?.textContent).toBe('Urgent');
		expect(cards.find((c) => c.textContent?.includes('Quiet'))!.querySelector('.urgent-chip')).toBeNull();
	});
});
