// @vitest-environment jsdom
// Opened by an external reference (`/board?story=SCRUM-11`), the drawer reads the activity log by the story's canonical
// key: the story endpoints resolve a reference, the activity log is keyed by the canonical key only.
import { flushSync, mount, tick, unmount } from 'svelte';
import { page } from '$app/state';
import { decisionHref } from '#lib/drawer.ts';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Story } from '#lib/api/types.ts';
import { activity, decision, goal, sprint, sprintView, story as storyDto } from '#lib/test/fixtures.ts';
import { failure, json } from '#lib/test/net.ts';
import { discardDraft } from '#lib/report.svelte.ts';
import { resetDecisionDraftsForTests } from '#lib/decisionDrafts.svelte.ts';
import StoryDrawer from './StoryDrawer.svelte';

vi.mock('$app/state', async () => ({
	page: (await import('#lib/test/net.ts')).fakePage({ url: '/board?story=SCRUM-11' }).page
}));
// A person at the keyboard: a refusal the server marks overridable raises the override prompt (kb/ADR-0018 R2).
vi.mock('#lib/me.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/me.svelte.ts')>()),
	isInteractive: () => true
}));

/** A move request's body, as the drawer sends it. */
type Moved = { to: Story['delivery']; resolution?: Story['resolution']; override_reason?: string };

const story = storyDto({ key: 'RV-5', external_ref: 'SCRUM-11', title: 'Imported story', rank: 1 });
const created = activity({
	id: 1,
	action: 'created',
	actor: 'dan@example.com',
	at: '2026-10-01T00:00:00Z',
	entity_key: 'RV-5',
	payload: null
});

/** The answer for each GET the drawer makes; the activity log knows only the canonical key. */
function answer(url: URL, method = 'GET'): unknown {
	if (method !== 'GET') throw new Error(`unexpected ${method} ${url.pathname}`);
	const p = url.pathname;
	if (p === '/api/v1/activity') return url.searchParams.get('entity_key') === 'RV-5' ? [created] : [];
	if (p.endsWith('/ready')) return { ready: true, reasons: [] };
	if (p.endsWith('/comments') || p.endsWith('/attachments') || p === '/api/v1/decisions' || p === '/api/v1/epics') return [];
	if (p === '/api/v1/initiatives') return [{ key: 'RV-1', title: 'Payments' }];
	if (p === '/api/v1/sprints') return { active: null, closed: [], draft: null };
	if (p.startsWith('/api/v1/stories/')) return story;
	throw new Error(`unexpected request ${p}`);
}

const net = vi.hoisted(() => ({
	requested: [] as URL[],
	/** Every request in order, with its method: a test asserts what a click sent. */
	sent: [] as { method: string; path: string }[],
	/** The JSON body of every write, in order. */
	bodies: [] as { path: string; body: unknown }[],
	answers: { of: (_url: URL, _method: string): unknown => null }
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(({ method, url, path, body }) => {
		net.requested.push(url);
		net.sent.push({ method, path });
		if (method !== 'GET') net.bodies.push({ path, body });
		return net.answers.of(url, method);
	});
	return orig();
});
net.answers.of = answer;

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	resetDecisionDraftsForTests();
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.requested.length = 0;
	net.sent.length = 0;
	net.bodies.length = 0;
	net.answers.of = answer;
	discardDraft();
});

describe('StoryDrawer Report action', () => {
	/** Waits until the drawer's load has sent its last request (the activity read is chained on the story read), so a
	 *  request from this test never lands in the next test's `net.requested` after afterEach cleared it. */
	const settled = () =>
		vi.waitFor(() => expect(net.requested.some((u) => u.pathname === '/api/v1/activity')).toBe(true));

	it('has no Report button of its own: the top bar stays usable above the drawer', async () => {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		await settled();
		const labels = [...document.body.querySelectorAll('header.top button')].map((b) => b.textContent?.trim());
		expect(labels).not.toContain('Report');
		expect(labels).not.toContain('Resume report');
		expect(document.body.querySelector('header.top button.close')).not.toBeNull(); // the header itself rendered
	});
});

describe('StoryDrawer decision readiness reasons', () => {
	it('links a decision reason to the decision drawer and leaves other reasons as text', async () => {
		net.answers.of = (url, method) =>
			url.pathname.endsWith('/ready')
				? { ready: false, reasons: [{ reason: 'decision_unchecked', decision_id: 7, title: 'Use Postgres' }, { reason: 'no_criteria' }] }
				: answer(url, method);
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const items = await vi.waitFor(() => {
			flushSync();
			const li = [...document.body.querySelectorAll('.readiness .reasons li')];
			expect(li).toHaveLength(2);
			return li;
		});
		const a = items[0].querySelector('a');
		expect(a?.getAttribute('href')).toBe(decisionHref(page.url, 7));
		expect(a?.getAttribute('href')).toBe('/board?decision=7');
		expect(a?.textContent).toContain('Use Postgres');
		expect(items[1].querySelector('a')).toBeNull();
	});
});

describe('StoryDrawer required decisions', () => {
	it('opens a decision in its drawer from its title', async () => {
		net.answers.of = (url, method) =>
			url.pathname === '/api/v1/decisions' ? [decision({ id: 7, on: 'RV-5', on_type: 'story', title: 'Use Postgres' })] : answer(url, method);
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const title = await vi.waitFor(() => {
			flushSync();
			const a = document.body.querySelector<HTMLAnchorElement>('.decisions a.title');
			expect(a).not.toBeNull();
			return a!;
		});
		expect(title.textContent).toBe('Use Postgres');
		expect(title.getAttribute('href')).toBe(decisionHref(page.url, 7));
		expect(title.getAttribute('href')).toBe('/board?decision=7');
	});
});

describe('StoryDrawer opened by an external reference', () => {
	it('shows the activity logged under the canonical key', async () => {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'SCRUM-11', onclose: () => {} } });
		// Activity sits behind the Discussion section's second tab, linked to its tabpanel.
		const tab = await vi.waitFor(() => {
			flushSync();
			const t = [...document.body.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find((b) => b.textContent === 'Activity');
			expect(t).toBeDefined();
			return t!;
		});
		// Both wrappers stay mounted, so the panel's text holds the activity either way: assert which one is shown.
		const wrappers = () => [...document.body.querySelectorAll<HTMLElement>('[role="tabpanel"] > div')];
		const [commentsWrap, activityWrap] = wrappers();
		expect(activityWrap.hidden).toBe(true);
		expect(commentsWrap.hidden).toBe(false);
		tab.click();
		await vi.waitFor(() => {
			flushSync();
			const panel = document.body.querySelector('[role="tabpanel"]');
			expect(panel?.getAttribute('aria-labelledby')).toBe(tab.id);
			expect(activityWrap.hidden).toBe(false);
			expect(commentsWrap.hidden).toBe(true);
			expect(activityWrap.textContent).toContain('created');
		});
		await tick();
		const activity = net.requested.filter((u) => u.pathname === '/api/v1/activity');
		expect(activity.map((u) => u.searchParams.get('entity_key'))).toEqual(['RV-5']);
	});

	it('copies the story\'s own key, not the reference it was opened by', async () => {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'SCRUM-11', onclose: () => {} } });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[data-testid="story-drawer"] header button[aria-label="Copy RV-5"]')).not.toBeNull();
		});
	});

	it('names the reference the story came from', async () => {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'SCRUM-11', onclose: () => {} } });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('[data-testid="story-drawer"] header')?.textContent).toContain(
				'from SCRUM-11'
			);
		});
	});
});

describe('StoryDrawer removes a block', () => {
	it('sends one DELETE for the edge and reloads the story', async () => {
		let blockers = ['RV-6'];
		net.answers.of = (url, method) => {
			if (method === 'DELETE' && url.pathname === '/api/v1/stories/RV-5/blocks/RV-6') {
				blockers = [];
				return {};
			}
			if (url.pathname === '/api/v1/stories/RV-5') return storyDto({ ...story, blocked_by: blockers });
			return answer(url, method);
		};
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const remove = await vi.waitFor(() => {
			flushSync();
			const b = document.body.querySelector<HTMLButtonElement>('button[aria-label="Remove block by RV-6"]');
			expect(b).not.toBeNull();
			return b!;
		});
		const before = net.sent.length;
		remove.click();
		remove.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.textContent).toContain('Nothing blocks this story.');
		});
		// The add form is closed, so focus lands on its "+ Add blocker" button.
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('drawer-blocker-edit'));
		const after = net.sent.slice(before);
		expect(after[0]).toEqual({ method: 'DELETE', path: '/api/v1/stories/RV-5/blocks/RV-6' });
		expect(after.filter((r) => r.method !== 'GET')).toHaveLength(1);
		expect(after.some((r) => r.method === 'GET' && r.path === '/api/v1/stories/RV-5')).toBe(true);
	});

	it('shows a failed removal and keeps the blocker', async () => {
		net.answers.of = (url, method) => {
			if (method === 'DELETE')
				return failure('no story RV-6', 404, 'not_found');
			if (url.pathname === '/api/v1/stories/RV-5') return storyDto({ ...story, blocked_by: ['RV-6'] });
			return answer(url, method);
		};
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const remove = await vi.waitFor(() => {
			flushSync();
			const b = document.body.querySelector<HTMLButtonElement>('button[aria-label="Remove block by RV-6"]');
			expect(b).not.toBeNull();
			return b!;
		});
		remove.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('.notice[role="status"]')?.textContent).toContain('no story RV-6');
		});
		const b = document.body.querySelector<HTMLButtonElement>('button[aria-label="Remove block by RV-6"]');
		expect(b).not.toBeNull();
		expect(b!.disabled).toBe(false);
	});
});

describe('StoryDrawer status button', () => {
	it('moves through the status button: done asks for the resolution in a dialog', async () => {
		net.answers.of = (url, method) =>
			url.pathname === '/api/v1/stories/RV-5' ? storyDto({ ...story, delivery: 'in_progress' }) : answer(url, method);
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const status = await vi.waitFor(() => {
			flushSync();
			const b = document.body.querySelector<HTMLButtonElement>('button[aria-label^="Move"][aria-haspopup="menu"]');
			expect(b).not.toBeNull();
			return b!;
		});
		expect(status.getAttribute('aria-label')).toBe('Move RV-5, now in progress');
		const before = net.sent.length;
		status.click();
		flushSync();
		await tick();
		const done = [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
			(b) => b.textContent === 'Move to done'
		)!;
		expect(done).toBeDefined();
		done.click();
		flushSync();
		const dialog = document.body.querySelector('dialog')!;
		expect(dialog.textContent).toContain('How was it resolved?');
		expect([...dialog.querySelectorAll('button')].map((b) => b.textContent)).toEqual(
			expect.arrayContaining(['completed', 'dropped'])
		);
		// Nothing was sent: done waits for the resolution (kb/ADR-0002 R4).
		expect(net.sent.slice(before).filter((r) => r.method !== 'GET')).toHaveLength(0);
	});
});

describe('StoryDrawer moves', () => {
	/** An in-progress RV-5 whose move endpoint answers with the moved story; later GETs see the new state. */
	function inProgress() {
		let current: Story = { ...story, delivery: 'in_progress' };
		net.answers.of = (url, method) => {
			if (method === 'POST' && url.pathname === '/api/v1/stories/RV-5/move') {
				const body = net.bodies.at(-1)!.body as Moved;
				current = { ...current, delivery: body.to, resolution: body.resolution ?? null, version: 2 };
				return current;
			}
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
	}

	async function open(): Promise<HTMLButtonElement> {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		return vi.waitFor(() => {
			flushSync();
			const b = document.body.querySelector<HTMLButtonElement>('button[aria-label^="Move"][aria-haspopup="menu"]');
			expect(b).not.toBeNull();
			return b!;
		});
	}

	async function pick(status: HTMLButtonElement, label: string) {
		status.click();
		flushSync();
		await tick();
		const item = [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(
			(b) => b.textContent === label
		);
		expect(item).toBeDefined();
		item!.click();
		flushSync();
	}

	it('posts done with the chosen resolution, reloads and returns focus to the status button', async () => {
		inProgress();
		const status = await open();
		await pick(status, 'Move to done');
		const completed = [...document.body.querySelectorAll<HTMLButtonElement>('dialog button')].find(
			(b) => b.textContent === 'completed'
		)!;
		const before = net.sent.length;
		completed.click();
		flushSync();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('button[aria-label^="Move"][aria-haspopup="menu"]')?.getAttribute('aria-label')).toBe(
				'Move RV-5, now done'
			);
		});
		expect(document.body.querySelector('dialog')).toBeNull();
		expect(net.bodies).toEqual([{ path: '/api/v1/stories/RV-5/move', body: { version: 1, to: 'done', resolution: 'completed' } }]);
		const after = net.sent.slice(before);
		expect(after[0]).toEqual({ method: 'POST', path: '/api/v1/stories/RV-5/move' });
		expect(after.some((r) => r.method === 'GET' && r.path === '/api/v1/stories/RV-5')).toBe(true);
		expect(document.activeElement).toBe(document.body.querySelector('button[aria-label^="Move"][aria-haspopup="menu"]'));
	});

	it('a refused move raises the override prompt; Move anyway re-posts with the reason and refocuses the status button', async () => {
		let current: Story = { ...story, delivery: 'todo' };
		net.answers.of = (url, method) => {
			if (method === 'POST' && url.pathname === '/api/v1/stories/RV-5/move') {
				const body = net.bodies.at(-1)!.body as Moved;
				if (!body.override_reason)
					return json({ outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: true }, 422);
				current = { ...current, delivery: body.to, version: 2 };
				return current;
			}
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
		const status = await open();
		await pick(status, 'Move to in progress');
		const dialog = await vi.waitFor(() => {
			flushSync();
			const d = document.body.querySelector('dialog');
			expect(d?.querySelector('h2')?.textContent).toBe('RV-5 is not ready for in progress');
			return d!;
		});
		expect(dialog.textContent).toContain('no acceptance criteria');
		const why = [...dialog.querySelectorAll('label')].find((l) => l.textContent?.trim().startsWith('Reason'))!.querySelector('textarea')!;
		why.value = 'Urgent fix';
		why.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		[...dialog.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent === 'Move anyway')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
			expect(status.getAttribute('aria-label')).toBe('Move RV-5, now in progress');
		});
		expect(net.bodies).toEqual([
			{ path: '/api/v1/stories/RV-5/move', body: { version: 1, to: 'in_progress' } },
			{ path: '/api/v1/stories/RV-5/move', body: { version: 1, to: 'in_progress', override_reason: 'Urgent fix' } }
		]);
		await vi.waitFor(() => expect(document.activeElement).toBe(status));
	});

	it('posts a move to testing without a resolution', async () => {
		inProgress();
		const status = await open();
		await pick(status, 'Move to testing');
		expect(document.body.querySelector('dialog')).toBeNull();
		await vi.waitFor(() => expect(net.bodies).toHaveLength(1));
		expect(net.bodies[0]).toEqual({ path: '/api/v1/stories/RV-5/move', body: { version: 1, to: 'testing' } });
		await vi.waitFor(() => {
			flushSync();
			expect(status.getAttribute('aria-label')).toBe('Move RV-5, now testing');
		});
	});

	it('offers a completed done story a resolution change, not a move to done', async () => {
		net.answers.of = (url, method) =>
			url.pathname === '/api/v1/stories/RV-5' ? storyDto({ ...story, delivery: 'done', resolution: 'completed' }) : answer(url, method);
		const status = await open();
		status.click();
		flushSync();
		await tick();
		const labels = [...document.body.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent);
		expect(labels).toContain('Change resolution');
		expect(labels).not.toContain('Move to done');
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent === 'Change resolution')!.click();
		flushSync();
		expect(document.body.querySelector('dialog')?.textContent).toContain('Change the resolution of RV-5');
	});
});

describe('StoryDrawer discussion tabs', () => {
	it('keeps a half-written comment across a switch to Activity and back', async () => {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const draft = await vi.waitFor(() => {
			flushSync();
			const t = document.body.querySelector<HTMLTextAreaElement>('textarea[aria-label="comment on RV-5"]');
			expect(t).not.toBeNull();
			return t!;
		});
		draft.value = 'half a thought';
		draft.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		const tab = (name: string) =>
			[...document.body.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find((b) => b.textContent === name)!;
		tab('Activity').click();
		flushSync();
		expect(draft.closest('[hidden]')).not.toBeNull();
		tab('Comments').click();
		flushSync();
		const again = document.body.querySelector<HTMLTextAreaElement>('textarea[aria-label="comment on RV-5"]')!;
		expect(again.value).toBe('half a thought');
		expect(again.closest('[hidden]')).toBeNull();
	});
});

describe('StoryDrawer reads as a view', () => {
	async function open(): Promise<HTMLElement> {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		return vi.waitFor(() => {
			flushSync();
			const d = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(d?.querySelector('.title')?.textContent).toBe('Imported story');
			return d!;
		});
	}
	const button = (d: HTMLElement, name: string) =>
		[...d.querySelectorAll<HTMLButtonElement>('button')].find((b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === name);

	it('opens with no field in edit mode and no decision or blocker form', async () => {
		const d = await open();
		// The comment composer (and its hidden image picker) is always there; no other field is.
		expect(d.querySelector('input:not([type="file"]), select, textarea:not([aria-label^="comment"])')).toBeNull();
		expect(d.querySelector('h4')).toBeNull();
		expect(button(d, '+ Require a decision')).toBeDefined();
		expect(button(d, '+ Add blocker')).toBeDefined();
		for (const what of ['title', 'acceptance criteria', 'description', 'owner', 'labels', 'epic'])
			expect(button(d, `Edit ${what}`), what).toBeDefined();
	});

	it('the pen opens the title editor; Save patches and returns to the view', async () => {
		let current: Story = story;
		net.answers.of = (url, method) => {
			if (method === 'PATCH') {
				current = { ...current, title: 'Renamed', version: 2 };
				return current;
			}
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
		const d = await open();
		button(d, 'Edit title')!.click();
		const input = await vi.waitFor(() => {
			const i = d.querySelector<HTMLInputElement>('#drawer-title');
			expect(document.activeElement).toBe(i);
			return i!;
		});
		input.value = 'Renamed';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		input.form!.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('#drawer-title')).toBeNull();
			expect(d.querySelector('.title')?.textContent).toBe('Renamed');
		});
		expect(net.bodies).toEqual([{ path: '/api/v1/stories/RV-5', body: { version: 1, title: 'Renamed' } }]);
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('drawer-title-edit'));
	});

	it('Escape cancels an open editor without saving or closing the drawer', async () => {
		const onclose = vi.fn();
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose } });
		const d = await vi.waitFor(() => {
			flushSync();
			const el = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(el?.querySelector('.title')).not.toBeNull();
			return el!;
		});
		button(d, 'Edit labels')!.click();
		await tick();
		const input = d.querySelector<HTMLInputElement>('#drawer-labels')!;
		input.value = 'half, typed';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
		flushSync();
		expect(d.querySelector('#drawer-labels')).toBeNull();
		expect(onclose).not.toHaveBeenCalled();
		expect(net.bodies).toEqual([]);
		button(d, 'Edit labels')!.click();
		flushSync();
		expect(d.querySelector<HTMLInputElement>('#drawer-labels')!.value).toBe('');
	});

	it('the criteria pen moves focus into the criteria editor', async () => {
		const d = await open();
		button(d, 'Edit acceptance criteria')!.click();
		await vi.waitFor(() => expect(document.activeElement?.getAttribute('aria-label')).toBe('add criterion to RV-5'));
	});

	it('a save that finishes after focus moved to another editor leaves focus there', async () => {
		let release!: () => void;
		let current: Story = story;
		net.answers.of = (url, method) => {
			if (method === 'PATCH')
				return new Promise((resolve) => {
					release = () => {
						current = { ...current, title: 'Renamed', version: 2 };
						resolve(current);
					};
				});
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
		const d = await open();
		button(d, 'Edit title')!.click();
		const title = await vi.waitFor(() => {
			const i = d.querySelector<HTMLInputElement>('#drawer-title');
			expect(document.activeElement).toBe(i);
			return i!;
		});
		title.value = 'Renamed';
		title.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		title.form!.requestSubmit();
		await vi.waitFor(() => expect(net.bodies).toHaveLength(1));
		button(d, 'Edit labels')!.click();
		const labels = await vi.waitFor(() => {
			const i = d.querySelector<HTMLInputElement>('#drawer-labels');
			expect(document.activeElement).toBe(i);
			return i!;
		});
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('.title')?.textContent).toBe('Renamed');
		});
		await tick();
		expect(document.activeElement).toBe(labels);
	});

	it('shows a story whose labels repeat', async () => {
		net.answers.of = (url, method) =>
			url.pathname === '/api/v1/stories/RV-5' ? storyDto({ ...story, labels: ['bug', 'bug'] }) : answer(url, method);
		const d = await open();
		expect([...d.querySelectorAll('.field .chip')].map((c) => c.textContent)).toEqual(['bug', 'bug']);
	});

	it('Save keeps a criterion typed but not added', async () => {
		let current: Story = story;
		net.answers.of = (url, method) => {
			if (method === 'PATCH') {
				current = { ...current, criteria: [{ text: 'typed only', done: false }], version: 2 };
				return current;
			}
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
		const d = await open();
		button(d, 'Edit acceptance criteria')!.click();
		flushSync();
		const i = d.querySelector<HTMLInputElement>('input[aria-label="add criterion to RV-5"]')!;
		i.value = 'typed only';
		i.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		button(d, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('.criteria-view')?.textContent).toContain('typed only');
		});
		expect(net.bodies).toEqual([
			{ path: '/api/v1/stories/RV-5', body: { version: 1, criteria: [{ text: 'typed only', done: false }] } }
		]);
	});

	it('the decision opener waits while a write is pending', async () => {
		let release!: () => void;
		net.answers.of = (url, method) => {
			if (method === 'PATCH') return new Promise((resolve) => (release = () => resolve({ ...story, title: 'Renamed', version: 2 })));
			return answer(url, method);
		};
		const d = await open();
		button(d, 'Edit title')!.click();
		flushSync();
		const t = d.querySelector<HTMLInputElement>('#drawer-title')!;
		t.value = 'Renamed';
		t.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		t.form!.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(button(d, '+ Require a decision')!.disabled).toBe(true);
		});
		await vi.waitFor(() => expect(net.bodies).toHaveLength(1));
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(button(d, '+ Require a decision')!.disabled).toBe(false);
		});
	});

	it('an editor is locked while its own save is pending, so nothing typed or reopened then is lost', async () => {
		let release!: () => void;
		let current: Story = story;
		net.answers.of = (url, method) => {
			if (method === 'PATCH')
				return new Promise((resolve) => {
					release = () => {
						current = { ...current, criteria: [{ text: 'first', done: false }], version: 2 };
						resolve(current);
					};
				});
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
		const d = await open();
		button(d, 'Edit acceptance criteria')!.click();
		flushSync();
		const adding = d.querySelector<HTMLInputElement>('input[aria-label="add criterion to RV-5"]')!;
		adding.value = 'first';
		adding.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		button(d, 'Save')!.click();
		await vi.waitFor(() => expect(net.bodies).toHaveLength(1));
		flushSync();
		expect(adding.disabled).toBe(true);
		expect(d.querySelector<HTMLInputElement>('input[aria-label="criterion 1"]')!.disabled).toBe(true);
		// Nor can it be cancelled and reopened: the save's reload would replace the reopened draft.
		expect(button(d, 'Cancel')!.disabled).toBe(true);
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('.criteria-view')?.textContent).toContain('first');
		});
	});

	it('Escape in the criteria or description editor cancels it, not the drawer', async () => {
		const onclose = vi.fn();
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose } });
		const d = await vi.waitFor(() => {
			flushSync();
			const el = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(el?.querySelector('.title')).not.toBeNull();
			return el!;
		});
		for (const [what, form] of [
			['acceptance criteria', '#drawer-criteria-form'],
			['description', '#drawer-description-form']
		]) {
			button(d, `Edit ${what}`)!.click();
			flushSync();
			button(d, 'Cancel')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
			flushSync();
			expect(d.querySelector(form), what).toBeNull();
		}
		expect(onclose).not.toHaveBeenCalled();
	});

	it('names the epic by its key and title in the header', async () => {
		net.answers.of = (url, method) => {
			if (url.pathname === '/api/v1/epics') return [{ key: 'RV-2', title: 'Migration findings' }];
			if (url.pathname === '/api/v1/stories/RV-5') return storyDto({ ...story, epic: 'RV-2' });
			return answer(url, method);
		};
		const d = await open();
		await vi.waitFor(() => {
			flushSync();
			const link = d.querySelector('header .epic-link')!;
			expect(link.querySelector('.key')?.textContent).toBe('RV-2');
			expect(link.textContent).toContain('Migration findings');
		});
	});
});

describe('StoryDrawer Goal field', () => {
	const goals = [goal({ id: 1, outcome: 'Ship the import' }), goal({ id: 2, outcome: 'Faster board' })];
	/** An active sprint with goals 1 and 2; the story serves `served` (the server's `story.goals`). */
	function serve(served: Story['goals']) {
		let current: Story = storyDto({ ...story, goals: served });
		net.answers.of = (url, method) => {
			if (url.pathname === '/api/v1/sprints') return { active: sprintView(sprint({ goals })), closed: [], draft: null };
			if (method === 'POST' && url.pathname === '/api/v1/goals/2/links') {
				current = { ...current, goals: [...current.goals, { id: 2, outcome: 'Faster board', via: 'story' }] };
				return goals[1];
			}
			if (method === 'DELETE' && url.pathname === '/api/v1/goals/1/links/RV-5') {
				current = { ...current, goals: current.goals.filter((g) => g.id !== 1) };
				return goals[0];
			}
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
	}
	async function open(): Promise<HTMLElement> {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		return vi.waitFor(() => {
			flushSync();
			const d = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(d?.querySelector('.title')).not.toBeNull();
			expect(d?.querySelector('#drawer-goal-edit')).not.toBeNull();
			return d!;
		});
	}
	const field = (d: HTMLElement) => d.querySelector('#drawer-goal-edit')!.closest('.field')!;

	it('the pen lists the active sprint goals; choosing one posts a link and shows it', async () => {
		serve([]);
		const d = await open();
		expect(field(d).textContent).toContain('none');
		d.querySelector<HTMLButtonElement>('#drawer-goal-edit')!.click();
		const select = await vi.waitFor(() => {
			const sel = d.querySelector<HTMLSelectElement>('#drawer-goal');
			expect(document.activeElement).toBe(sel);
			return sel!;
		});
		expect([...select.options].map((o) => o.textContent?.trim())).toEqual(['choose a goal', 'Ship the import', 'Faster board']);
		select.value = '2';
		select.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		select.form!.requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('#drawer-goal')).toBeNull();
			expect(field(d).textContent).toContain('Faster board');
		});
		expect(net.bodies).toEqual([{ path: '/api/v1/goals/2/links', body: { items: ['RV-5'] } }]);
		await vi.waitFor(() => expect(document.activeElement?.id).toBe('drawer-goal-edit'));
	});

	it('the clear control unlinks a goal the story itself links', async () => {
		serve([{ id: 1, outcome: 'Ship the import', via: 'story' }]);
		const d = await open();
		expect(field(d).textContent).toContain('Ship the import');
		const unlink = field(d).querySelector<HTMLButtonElement>('button[aria-label="Unlink RV-5 from Ship the import"]')!;
		unlink.click();
		await vi.waitFor(() => {
			flushSync();
			expect(field(d).textContent).not.toContain('Ship the import');
		});
		expect(net.sent.filter((r) => r.method !== 'GET')).toEqual([{ method: 'DELETE', path: '/api/v1/goals/1/links/RV-5' }]);
	});

	it('a goal served through the epic says so and has no clear control here (the sprint page unlinks the epic)', async () => {
		serve([{ id: 1, outcome: 'Ship the import', via: 'epic' }]);
		const d = await open();
		expect(field(d).textContent).toContain('Ship the import');
		expect(field(d).textContent).toContain('via epic');
		expect(field(d).querySelector('button[aria-label^="Unlink"]')).toBeNull();
	});

	it('a failed GET /sprints leaves the drawer without the Goal field, not without the story', async () => {
		net.answers.of = (url, method) => (url.pathname === '/api/v1/sprints' ? failure('sprints are down') : answer(url, method));
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const d = await vi.waitFor(() => {
			flushSync();
			const el = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(el?.querySelector('.title')?.textContent).toBe('Imported story');
			return el!;
		});
		expect(d.querySelector('#drawer-goal-edit')).toBeNull();
		expect(d.textContent).not.toContain('Could not');
	});

	it('is absent with no active sprint', async () => {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		const d = await vi.waitFor(() => {
			flushSync();
			const el = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(el?.querySelector('.title')).not.toBeNull();
			return el!;
		});
		expect(d.querySelector('#drawer-goal-edit')).toBeNull();
		expect([...d.querySelectorAll('.fields .name')].map((n) => n.textContent)).toEqual(['Owner', 'Labels', 'Epic']);
	});
});

describe('StoryDrawer urgent mark', () => {
	const mark = (over: Partial<NonNullable<Story['urgent_mark']>> = {}) => ({
		at: '2026-11-07T00:00:00Z',
		by: 'dana@example.com',
		reason: 'customer waiting',
		expires_at: '2099-01-01T00:00:00Z',
		active: true,
		done: false,
		...over
	});
	/** Serves `current` for the story, applies the urgent POST and DELETE to it. */
	function serve(current: Story) {
		net.answers.of = (url, method) => {
			if (url.pathname === '/api/v1/stories/RV-5/urgent') {
				current =
					method === 'POST'
						? { ...current, urgent_mark: mark(), urgent_sources: [{ key: 'RV-5', item: 'story' }] }
						: { ...current, urgent_mark: null, urgent_sources: [] };
				return current;
			}
			if (url.pathname === '/api/v1/epics') return [{ key: 'RV-2', title: 'Migration findings' }];
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
	}
	async function open(): Promise<HTMLElement> {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		return vi.waitFor(() => {
			flushSync();
			const el = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(el?.querySelector('.title')).not.toBeNull();
			return el!;
		});
	}
	const items = () => [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
	async function openMenu(d: HTMLElement) {
		d.querySelector<HTMLButtonElement>('button[aria-label="More actions for RV-5"]')!.click();
		flushSync();
		await tick();
	}

	it('offers Mark urgent… on an unmarked story and posts the reason from its dialog', async () => {
		serve(storyDto({ ...story }));
		const d = await open();
		await openMenu(d);
		expect(items().map((b) => b.textContent)).toEqual(['Mark urgent…', 'Promise by…']);
		items()[0].click();
		flushSync();
		await tick();
		const dialog = document.body.querySelector('dialog')!;
		const input = dialog.querySelector('input')!;
		input.value = 'customer waiting';
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		[...dialog.querySelectorAll('button')].find((b) => b.textContent === 'Mark urgent')!.click();
		await vi.waitFor(() => expect(net.bodies.at(-1)).toEqual({ path: '/api/v1/stories/RV-5/urgent', body: { reason: 'customer waiting' } }));
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
			expect(d.querySelector('[data-testid="urgent-line"]')?.textContent).toContain('marked by dana');
		});
	});

	it('shows the own and the derived parts under the title, and clears with DELETE', async () => {
		serve(
			storyDto({
				...story,
				epic: 'RV-2',
				urgent_mark: mark(),
				urgent_sources: [
					{ key: 'RV-5', item: 'story' },
					{ key: 'RV-2', item: 'epic' }
				]
			})
		);
		const d = await open();
		const line = await vi.waitFor(() => {
			flushSync();
			const l = d.querySelector('[data-testid="urgent-line"]');
			expect(l).not.toBeNull();
			return l!;
		});
		expect(line.textContent).toContain("'customer waiting'");
		await vi.waitFor(() => {
			flushSync();
			expect(line.textContent).toContain('via epic RV-2 Migration findings');
		});
		await openMenu(d);
		expect(items().map((b) => b.textContent)).toEqual(['Mark urgent…', 'Clear urgent mark', 'Promise by…']);
		items()[1].click();
		await vi.waitFor(() => expect(net.sent.some((s) => s.method === 'DELETE' && s.path === '/api/v1/stories/RV-5/urgent')).toBe(true));
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('[data-testid="urgent-line"]')).toBeNull();
		});
	});

	it('shows a failed mark in the dialog, not the drawer notice', async () => {
		serve(storyDto({ ...story }));
		const base = net.answers.of;
		net.answers.of = (url, method) =>
			method === 'POST' && url.pathname.endsWith('/urgent') ? failure('boom') : base(url, method);
		const d = await open();
		await openMenu(d);
		items()[0].click();
		flushSync();
		await tick();
		[...document.body.querySelectorAll<HTMLButtonElement>('dialog button')].find((b) => b.textContent === 'Mark urgent')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog [role="alert"]')).not.toBeNull();
		});
		expect(document.body.querySelector('dialog [role="alert"]')!.textContent).toContain('boom');
		expect(d.querySelector('.notice')).toBeNull();
	});

	it('a mark on a done story reads "Urgent mark · done" with Clear and no chip', async () => {
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed', urgent_mark: mark({ done: true }), urgent_sources: [] }));
		const d = await open();
		const line = await vi.waitFor(() => {
			flushSync();
			const l = d.querySelector('[data-testid="urgent-done"]');
			expect(l).not.toBeNull();
			return l!;
		});
		expect(line.textContent).toContain('Urgent mark · done · expires');
		expect(d.querySelector('[data-testid="urgent-line"]')).toBeNull();
		await openMenu(d);
		expect(items().map((b) => b.textContent)).toEqual(['Clear urgent mark']);
		items()[0].click();
		await vi.waitFor(() => expect(net.sent.some((s) => s.method === 'DELETE' && s.path === '/api/v1/stories/RV-5/urgent')).toBe(true));
	});

	it('a done and no-longer-active mark shows only the done line: no expired line, one Clear', async () => {
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed', urgent_mark: mark({ done: true, active: false }), urgent_sources: [] }));
		const d = await open();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('[data-testid="urgent-done"]')).not.toBeNull();
		});
		expect(d.querySelector('[data-testid="urgent-expired"]')).toBeNull();
		expect(d.querySelector('button[aria-label="Re-mark RV-5"]')).toBeNull();
		expect(d.querySelectorAll('button[aria-label="Clear urgent mark on RV-5"]')).toHaveLength(1);
		// A same-labelled button elsewhere on the page must not take the focus.
		const other = document.createElement('button');
		other.setAttribute('aria-label', 'More actions for RV-5');
		document.body.prepend(other);
		d.querySelector<HTMLButtonElement>('button[aria-label="Clear urgent mark on RV-5"]')!.focus();
		d.querySelector<HTMLButtonElement>('button[aria-label="Clear urgent mark on RV-5"]')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('[data-testid="urgent-done"]')).toBeNull();
			expect(document.activeElement).toBe(d.querySelector('h2'));
		});
		expect(d.contains(document.activeElement)).toBe(true);
	});

	it('after a re-mark the focus returns to the drawer actions button, not another with the same label', async () => {
		serve(storyDto({ ...story, urgent_mark: mark({ active: false, expires_at: '2026-11-01T00:00:00Z' }), urgent_sources: [] }));
		const d = await open();
		const remark = await vi.waitFor(() => {
			flushSync();
			const b = d.querySelector<HTMLButtonElement>('button[aria-label="Re-mark RV-5"]');
			expect(b).not.toBeNull();
			return b!;
		});
		const other = document.createElement('button');
		other.setAttribute('aria-label', 'More actions for RV-5');
		document.body.prepend(other);
		remark.focus();
		remark.click();
		flushSync();
		await tick();
		[...document.body.querySelectorAll<HTMLButtonElement>('dialog button')].find((b) => b.textContent === 'Mark urgent')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
			expect(document.activeElement).toBe(d.querySelector('[data-control="story-actions"]'));
		});
		expect(document.activeElement).not.toBe(other);
	});

	it('hides Mark urgent… on a done story', async () => {
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed' }));
		const d = await open();
		expect(d.querySelector('[data-control="story-actions"]')).toBeNull();
	});

	it('titles an initiative source by the initiatives list', async () => {
		serve(storyDto({ ...story, urgent_mark: null, urgent_sources: [{ key: 'RV-1', item: 'initiative' }] }));
		const d = await open();
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('[data-testid="urgent-line"]')?.textContent).toContain('via initiative RV-1 Payments');
		});
	});

	it('a failed clear keeps the focus on the Clear button', async () => {
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed', urgent_mark: mark({ done: true }), urgent_sources: [] }));
		const base = net.answers.of;
		net.answers.of = (url, method) => (method === 'DELETE' ? failure('boom') : base(url, method));
		const d = await open();
		const clear = await vi.waitFor(() => {
			flushSync();
			const b = d.querySelector<HTMLButtonElement>('button[aria-label="Clear urgent mark on RV-5"]');
			expect(b).not.toBeNull();
			return b!;
		});
		clear.focus();
		clear.click();
		clear.blur();
		await vi.waitFor(() => {
			flushSync();
			expect(d.textContent).toContain('boom');
			expect(document.activeElement).toBe(d.querySelector('button[aria-label="Clear urgent mark on RV-5"]'));
		});
	});
});

describe('StoryDrawer promise', () => {
	const fact = (on: string, to: string | null = 'Acme') => ({ on, to, by: 'dana@example.com', at: '2026-11-01T09:00:00Z' });
	const own = (on: string, to: string | null = 'Acme'): NonNullable<Story['promise']> => ({
		own: fact(on, to),
		epic: null,
		on,
		holder: 'own',
		state: on < '2026-11-12' ? 'overdue' : 'open',
		due: on < '2026-11-12' ? null : 'later',
		days: on < '2026-11-12' ? 7 : null
	});
	/** Serves `current` for the story and a sprint list with `today`; applies the promise POST and DELETE to it. */
	function serve(current: Story, failPost = false, { failDelete = false, failSprints = false } = {}) {
		net.answers.of = (url, method) => {
			if (url.pathname === '/api/v1/stories/RV-5/promise') {
				if (method === 'POST' && failPost) return failure('boom');
				if (method === 'DELETE' && failDelete) return failure('no clearing');
				const body = net.bodies.at(-1)?.body as { on?: string } | undefined;
				const cleared = method === 'DELETE' && current.promise?.own != null;
				current = { ...current, promise: method === 'POST' ? own(body?.on ?? '2026-11-24') : null };
				return method === 'DELETE' ? { cleared, story: current } : current;
			}
			if (url.pathname === '/api/v1/sprints') {
				if (failSprints) return failure('sprints are down');
				return { active: sprintView(sprint({ ends_on: '2026-11-24' })), closed: [], draft: sprint({ id: 2, state: 'draft', ends_on: '2026-12-08' }), today: '2026-11-12' };
			}
			if (url.pathname === '/api/v1/stories/RV-5') return current;
			return answer(url, method);
		};
	}
	async function open(): Promise<HTMLElement> {
		app = mount(StoryDrawer, { target: document.body, props: { storyKey: 'RV-5', onclose: () => {} } });
		return vi.waitFor(() => {
			flushSync();
			const el = document.body.querySelector<HTMLElement>('[data-testid="story-drawer"]');
			expect(el?.querySelector('.title')).not.toBeNull();
			return el!;
		});
	}
	const items = () => [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
	async function openMenu(d: HTMLElement) {
		d.querySelector<HTMLButtonElement>('button[aria-label="More actions for RV-5"]')!.click();
		flushSync();
		await tick();
	}
	const dialogButton = (label: string) =>
		[...document.body.querySelectorAll<HTMLButtonElement>('dialog button')].find((b) => b.textContent?.trim() === label);

	it('promises by a sprint shortcut from the server\'s ends and shows the line', async () => {
		serve(storyDto({ ...story }));
		const d = await open();
		await openMenu(d);
		items().find((b) => b.textContent === 'Promise by…')!.click();
		flushSync();
		await tick();
		expect(document.body.querySelector('dialog')?.textContent).toContain('Promise RV-5 by…');
		dialogButton('End of this sprint (24 Nov)')!.click();
		flushSync();
		dialogButton('Promise')!.click();
		await vi.waitFor(() => expect(net.bodies.at(-1)).toEqual({ path: '/api/v1/stories/RV-5/promise', body: { shortcut: 'this_sprint' } }));
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
			expect(d.querySelector('[data-testid="promise-line"]')?.textContent).toContain('Promised by 24 Nov 2026 to Acme · set by dana');
			// The dialog is gone: the keyboard lands on the drawer's actions menu.
			expect(document.activeElement).toBe(d.querySelector('[data-control="story-actions"]'));
		});
	});

	it('changes the recipient of an overdue promise without a date, and clears with DELETE', async () => {
		serve(storyDto({ ...story, promise: own('2026-11-05') }));
		const d = await open();
		await openMenu(d);
		expect(items().map((b) => b.textContent)).toEqual(['Mark urgent…', 'Change promise…', 'Clear promise']);
		items()[1].click();
		flushSync();
		await tick();
		const to = document.body.querySelector<HTMLInputElement>('dialog input[name="to"]')!;
		to.value = 'Beta';
		to.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		dialogButton('Save')!.click();
		await vi.waitFor(() => expect(net.bodies.at(-1)).toEqual({ path: '/api/v1/stories/RV-5/promise', body: { to: 'Beta' } }));
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
		});
		await openMenu(d);
		items().find((b) => b.textContent === 'Clear promise')!.click();
		await vi.waitFor(() => expect(net.sent.some((s) => s.method === 'DELETE' && s.path === '/api/v1/stories/RV-5/promise')).toBe(true));
		await vi.waitFor(() => {
			flushSync();
			expect(d.querySelector('[data-testid="promise-line"]')).toBeNull();
			expect(document.activeElement).toBe(d.querySelector('[data-control="story-actions"]'));
		});
	});

	it('a done story offers only Clear promise', async () => {
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed', promise: { ...own('2026-11-20'), state: 'kept', due: null } }));
		const d = await open();
		await openMenu(d);
		expect(items().map((b) => b.textContent)).toEqual(['Clear promise']);
	});

	it("asks before clearing a done story's promise: Cancel sends nothing, a failure stays in the question", async () => {
		const deletes = () => net.sent.filter((s) => s.method === 'DELETE' && s.path === '/api/v1/stories/RV-5/promise').length;
		const kept = { ...own('2026-11-20'), state: 'kept' as const, due: null };
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed', promise: kept }), false, { failDelete: true });
		const d = await open();
		await openMenu(d);
		items().find((b) => b.textContent === 'Clear promise')!.click();
		flushSync();
		await tick();
		const dialog = () => document.body.querySelector('dialog');
		expect(dialog()?.querySelector('h2')?.textContent).toBe('Clear the promise on RV-5?');
		expect(dialog()?.textContent).toContain('cannot be set again');
		// Focus starts on the safe choice.
		expect(document.activeElement).toBe(dialogButton('Cancel'));
		dialogButton('Cancel')!.click();
		flushSync();
		expect(dialog()).toBeNull();
		expect(deletes()).toBe(0);

		await openMenu(d);
		items().find((b) => b.textContent === 'Clear promise')!.click();
		flushSync();
		await tick();
		// The story has no epic promise: the text says its own promise goes and the story leaves the sprint's list.
		const text = dialog()?.querySelector('[data-testid="clear-promise-text"]')?.textContent?.replace(/\s+/g, ' ').trim();
		expect(text).toBe('RV-5 is done, so its own promise cannot be set again once cleared, and the story leaves the promise list of the sprint its date falls in.');
		// A browser drops the focus of a button that becomes disabled; after the failure it lands on Cancel again.
		const confirm = dialogButton('Clear promise')!;
		confirm.focus();
		confirm.click();
		confirm.blur();
		await vi.waitFor(() => {
			flushSync();
			expect(dialog()?.querySelector('[role="alert"]')?.textContent).toContain('no clearing');
			expect(document.activeElement).toBe(dialogButton('Cancel'));
		});
		expect(deletes()).toBe(1);
		expect(d.querySelector('.notice, [role="alert"]')).toBeNull();
	});

	it("asking before clearing a done story's own promise under a promised epic says it stays listed under the epic's date", async () => {
		const both: NonNullable<Story['promise']> = {
			...own('2026-11-20'),
			epic: { key: 'RV-1', fact: fact('2026-11-24', 'Beta') },
			state: 'kept',
			due: null
		};
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed', promise: both }));
		const d = await open();
		await openMenu(d);
		items().find((b) => b.textContent === 'Clear promise')!.click();
		flushSync();
		await tick();
		const text = document.body.querySelector('dialog [data-testid="clear-promise-text"]')?.textContent?.replace(/\s+/g, ' ').trim();
		expect(text).toBe("RV-5 is done, so its own promise cannot be set again once cleared. It stays listed under epic RV-1's date.");
		expect(text).not.toContain('leaves');
	});

	it("clears a done story's promise once confirmed", async () => {
		const kept = { ...own('2026-11-20'), state: 'kept' as const, due: null };
		serve(storyDto({ ...story, delivery: 'done', resolution: 'completed', promise: kept }));
		const d = await open();
		await openMenu(d);
		items().find((b) => b.textContent === 'Clear promise')!.click();
		flushSync();
		await tick();
		dialogButton('Clear promise')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog')).toBeNull();
			expect(d.querySelector('[data-testid="promise-line"]')).toBeNull();
		});
		expect(net.sent.filter((s) => s.method === 'DELETE' && s.path === '/api/v1/stories/RV-5/promise')).toHaveLength(1);
	});

	it('says sprint shortcuts are unavailable when the sprints could not be read', async () => {
		serve(storyDto({ ...story }), false, { failSprints: true });
		const d = await open();
		await openMenu(d);
		items().find((b) => b.textContent === 'Promise by…')!.click();
		flushSync();
		await tick();
		expect(document.body.querySelector('[data-testid="promise-shortcuts"]')).toBeNull();
		expect(document.body.querySelector('[data-testid="promise-shortcuts-unavailable"]')?.textContent).toBe('Sprint shortcuts are unavailable.');
	});

	it('shows a failed save in the dialog, not the drawer notice', async () => {
		serve(storyDto({ ...story }), true);
		const d = await open();
		await openMenu(d);
		items().find((b) => b.textContent === 'Promise by…')!.click();
		flushSync();
		await tick();
		dialogButton('End of next sprint (8 Dec)')!.click();
		flushSync();
		dialogButton('Promise')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('dialog [role="alert"]')?.textContent).toContain('boom');
		});
		expect(d.querySelector('.notice, [role="alert"]')).toBeNull();
	});
});
