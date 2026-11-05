// @vitest-environment jsdom
// The board's sprint pieces: the goals strip filters the cards by the goals the server says they serve, and
// a start that serves no goal shows the nudge bar after the move lands, keyed on the move's reply, never the card before
// it. The move never depends on goals (kb/ADR-0014 R4): "Upkeep" writes nothing.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { page } from '$app/state';
import type { Sprints, Story, StoryWithReadiness } from '#lib/api/types.ts';
import { goal, sprint, sprintView, withReadiness } from '#lib/test/fixtures.ts';
import { failure, json, type Call } from '#lib/test/net.ts';
import { RULE } from '#lib/sprint.ts';
import Page from './+page.svelte';

vi.mock('$app/state', async () => {
	const { SvelteURL } = await import('svelte/reactivity');
	return { page: { url: new SvelteURL('http://localhost/board') } };
});
vi.mock('$app/navigation', () => ({
	goto: vi.fn(async (url: URL) => {
		((await import('$app/state')).page.url as URL).href = url.href;
	})
}));
vi.mock('#lib/me.svelte.ts', async () => ({
	session: await (async () => {
		const { config, me } = await import('#lib/test/fixtures.ts');
		return { me: me({ email: 'me@example.com' }), config: config({ done_window_days: 14 }) };
	})(),
	isInteractive: () => true,
	isAdmin: () => false,
	load: () => Promise.resolve()
}));
const live = vi.hoisted(() => ({ fns: [] as (() => void)[] }));
vi.mock('#lib/events.svelte.ts', () => ({
	live: { connected: false, lastId: 0, version: 0, latest: null, recent: [] },
	onLive: (fn: () => void) => live.fns.push(fn),
	freshNote: () => null
}));

const net = vi.hoisted(() => ({
	/** Every write, in order. */
	writes: [] as { method: string; path: string; body: unknown }[],
	handler: (_c: { method: string; url: URL; path: string; body: unknown }): unknown => null
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch((c) => {
		if (c.method !== 'GET') net.writes.push({ method: c.method, path: c.path, body: c.body });
		return net.handler(c);
	});
	return orig();
});

const ref = (id: number) => ({ id, outcome: id === 1 ? 'Ship the import' : 'Faster board', via: 'story' as const });

/** What the fake server holds: the stories, the sprints answer, and how a move answers. */
let stories: StoryWithReadiness[];
/** null answers `GET /sprints` with a server error. */
let sprints: Sprints | null;
/** The goals the move's reply says the moved story serves (the server computes them, not the card). */
let replyGoals: Story['goals'];
/** The first move is refused for readiness; the re-post with a reason lands. */
let needsOverride: boolean;

function serve(c: Call): unknown {
	const { method, path } = c;
	if (method === 'GET' && path === '/api/v1/stories') return stories;
	if (method === 'GET' && path === '/api/v1/epics') return [];
	if (method === 'GET' && path === '/api/v1/sprints') return sprints ?? failure('sprints are down');
	const moved = path.match(/^\/api\/v1\/stories\/([^/]+)\/move$/);
	if (method === 'POST' && moved) {
		const body = c.body as { to: Story['delivery']; override_reason?: string };
		if (needsOverride && !body.override_reason)
			return json({ outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: true }, 422);
		const row = stories.find((s) => s.story.key === moved[1])!;
		const story = { ...row.story, delivery: body.to, goals: replyGoals, version: row.story.version + 1 };
		stories = stories.map((s) => (s.story.key === story.key ? { ...s, story } : s));
		return story;
	}
	const linked = path.match(/^\/api\/v1\/goals\/(\d+)\/links$/);
	if (method === 'POST' && linked) return goal({ id: Number(linked[1]), links: (c.body as { items: string[] }).items });
	throw new Error(`unexpected ${method} ${path}`);
}

const view = () =>
	sprintView(sprint({ goals: [goal({ id: 1, outcome: 'Ship the import', confidence: 'at_risk' }), goal({ id: 2, outcome: 'Faster board' })] }), {
		counts: {
			completed: 3,
			upkeep_completed: 1,
			goals: [
				{ goal_id: 1, completed: 2, in_flight: 1, todo: 4, in_review: 0, blocked_decisions: 0 },
				{ goal_id: 2, completed: 0, in_flight: 0, todo: 1, in_review: 0, blocked_decisions: 0 }
			]
		}
	});

beforeEach(() => {
	stories = [
		withReadiness({ key: 'ABC-1', title: 'Import epics', goals: [ref(1)] }),
		withReadiness({ key: 'ABC-2', title: 'Started for goal 1', delivery: 'in_progress', goals: [ref(1)] }),
		withReadiness({ key: 'ABC-3', title: 'Board work', goals: [ref(2)] }),
		withReadiness({ key: 'ABC-4', title: 'Fix a typo', kind: 'chore' }),
		withReadiness({ key: 'ABC-5', title: 'Started upkeep', delivery: 'in_progress' }),
		withReadiness({ key: 'ABC-6', title: 'Started for goal 2', delivery: 'testing', goals: [ref(2)] })
	];
	sprints = { active: view(), closed: [], draft: null };
	replyGoals = [];
	needsOverride = false;
	net.handler = serve;
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.writes.length = 0;
	live.fns.length = 0;
	(page.url as URL).href = 'http://localhost/board';
});

const q = <T extends Element = HTMLElement>(sel: string) => document.body.querySelector<T>(sel);
const shownKeys = () => [...document.body.querySelectorAll<HTMLElement>('[data-testid^="card-"]')].map((c) => c.dataset.testid!.slice(5));
const chip = (id: number | 'upkeep') => q<HTMLButtonElement>(`[data-testid="goal-chip-${id}"]`)!;
const nudge = (key: string) => q(`li[data-key="${key}"] [data-testid="nudge"]`);
/** Lets every request in flight answer and render, so a write that should not happen would have been sent. */
async function settle() {
	await new Promise((r) => setTimeout(r, 50));
	flushSync();
}
const ALL = ['ABC-1', 'ABC-2', 'ABC-3', 'ABC-4', 'ABC-5', 'ABC-6'];
const buttonIn = (root: Element, text: string) =>
	[...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === text)!;

async function mounted() {
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(q('[data-testid="card-ABC-1"]')).not.toBeNull();
		expect(q('[data-testid="goals-strip"]')).not.toBeNull();
	});
}

/** Drags `key`'s card onto `col`, as a pointer drop does (jsdom has no DragEvent; the handlers read no data from it). */
function drop(key: string, col: string) {
	q(`[data-testid="card-${key}"]`)!.dispatchEvent(new Event('dragstart', { bubbles: true }));
	q(`[data-testid="column-${col}"]`)!.dispatchEvent(new Event('drop', { bubbles: true, cancelable: true }));
}

describe('board goals strip', () => {
	it('shows one chip per goal with its counts, the membership rule and a labelled confidence dot, then Upkeep', async () => {
		await mounted();
		const strip = q('[data-testid="goals-strip"]')!;
		expect(strip.getAttribute('title')).toBe(RULE);
		const chips = [...strip.querySelectorAll('button')];
		expect(chips.map((b) => b.dataset.testid)).toEqual(['goal-chip-1', 'goal-chip-2', 'goal-chip-upkeep']);
		expect(chip(1).textContent).toContain('Ship the import');
		expect(chip(1).textContent).toContain('2 / 1 / 4');
		expect(chip(1).querySelector('[role="img"]')?.getAttribute('aria-label')).toBe('confidence: at risk');
		expect(chip(2).querySelector('[role="img"]')?.getAttribute('aria-label')).toBe('confidence: not recorded');
		expect(chip('upkeep').textContent?.trim()).toBe('Upkeep');
		// kb/ADR-0014 R2: the rule is a visible line where counts show, not only a tooltip.
		expect(strip.querySelector('.rule')?.textContent).toBe(RULE);
		expect(chips.every((b) => b.getAttribute('aria-pressed') === 'false')).toBe(true);
	});

	it('a goal chip shows only its stories, each in its own column; clicking it again clears the filter', async () => {
		await mounted();
		chip(1).click();
		await vi.waitFor(() => {
			flushSync();
			expect(shownKeys().sort()).toEqual(['ABC-1', 'ABC-2']);
		});
		expect(page.url.searchParams.get('goal')).toBe('1');
		expect(chip(1).getAttribute('aria-pressed')).toBe('true');
		// The started goal story stays in its delivery column: the strip filters, it never moves cards; started stories that
		// serve no goal or another goal are hidden like todo ones.
		expect(q('[data-testid="column-in_progress"] [data-testid="card-ABC-2"]')).not.toBeNull();
		expect(q('[data-testid="card-ABC-5"]')).toBeNull();
		expect(q('[data-testid="card-ABC-6"]')).toBeNull();
		expect(q('[data-testid="column-todo"] [data-testid="card-ABC-1"]')).not.toBeNull();

		chip(1).click();
		await vi.waitFor(() => {
			flushSync();
			expect(shownKeys().sort()).toEqual(ALL);
		});
		expect(page.url.searchParams.has('goal')).toBe(false);
	});

	it('Upkeep shows only the stories that serve no goal', async () => {
		await mounted();
		chip('upkeep').click();
		await vi.waitFor(() => {
			flushSync();
			expect(shownKeys().sort()).toEqual(['ABC-4', 'ABC-5']);
		});
		expect(page.url.searchParams.get('goal')).toBe('none');
	});

	it('ignores ?goal= without an active sprint, so a stale filter hides nothing', async () => {
		sprints = { active: null, closed: [], draft: null };
		(page.url as URL).href = 'http://localhost/board?goal=1';
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(shownKeys().sort()).toEqual(ALL);
		});
	});

	it('ignores ?goal= naming no goal of the active sprint', async () => {
		(page.url as URL).href = 'http://localhost/board?goal=99';
		await mounted();
		expect(shownKeys().sort()).toEqual(ALL);
		expect(chip(1).getAttribute('aria-pressed')).toBe('false');
	});

	it('a failed GET /sprints leaves the board without its strip, not without its cards', async () => {
		sprints = null;
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(shownKeys().sort()).toEqual(ALL);
		});
		expect(q('[data-testid="goals-strip"]')).toBeNull();
		expect(document.body.textContent).not.toContain('Could not load the board');
	});

	it('is absent with no active sprint, and the board still loads', async () => {
		sprints = { active: null, closed: [], draft: null };
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(q('[data-testid="card-ABC-1"]')).not.toBeNull();
		});
		expect(q('[data-testid="goals-strip"]')).toBeNull();
	});
});

describe('board goal chip on a card', () => {
	it('names the first goal the story serves beside its epic', async () => {
		await mounted();
		expect(q('[data-testid="card-ABC-1"] .goal')?.textContent).toBe('goal: Ship the import');
		expect(q('[data-testid="card-ABC-4"] .goal')).toBeNull();
	});
});

describe('board start nudge', () => {
	it('after a todo card serving no goal is dropped into in progress, asks which goal; a goal posts one link', async () => {
		await mounted();
		drop('ABC-4', 'in_progress');
		const bar = await vi.waitFor(() => {
			flushSync();
			const b = nudge('ABC-4');
			expect(b).not.toBeNull();
			return b!;
		});
		expect(q('[data-testid="column-in_progress"] li[data-key="ABC-4"] [data-testid="nudge"]')).toBe(bar);
		expect(bar.textContent).toContain('Serving which goal?');
		expect([...bar.querySelectorAll('button')].map((b) => b.textContent?.trim())).toEqual(['Ship the import', 'Faster board', 'Upkeep']);
		// The bar is not a dialog: nothing is modal and the move already landed.
		expect(bar.closest('dialog')).toBeNull();
		expect(net.writes).toEqual([{ method: 'POST', path: '/api/v1/stories/ABC-4/move', body: { version: 1, to: 'in_progress' } }]);

		buttonIn(bar, 'Faster board').click();
		await vi.waitFor(() => {
			flushSync();
			expect(nudge('ABC-4')).toBeNull();
		});
		expect(net.writes.slice(1)).toEqual([{ method: 'POST', path: '/api/v1/goals/2/links', body: { items: ['ABC-4'] } }]);
	});

	it('Upkeep dismisses the bar and writes nothing', async () => {
		await mounted();
		drop('ABC-4', 'in_progress');
		const bar = await vi.waitFor(() => {
			flushSync();
			expect(nudge('ABC-4')).not.toBeNull();
			return nudge('ABC-4')!;
		});
		buttonIn(bar, 'Upkeep').click();
		await settle();
		expect(nudge('ABC-4')).toBeNull();
		expect(net.writes.map((w) => w.path)).toEqual(['/api/v1/stories/ABC-4/move']);
	});

	it('Escape dismisses the bar and writes nothing', async () => {
		await mounted();
		drop('ABC-4', 'in_progress');
		await vi.waitFor(() => {
			flushSync();
			expect(nudge('ABC-4')).not.toBeNull();
		});
		// An Escape in a form field is the field's own: the bar stays.
		q('select')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		flushSync();
		expect(nudge('ABC-4')).not.toBeNull();
		document.body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		await settle();
		expect(nudge('ABC-4')).toBeNull();
		expect(net.writes.map((w) => w.path)).toEqual(['/api/v1/stories/ABC-4/move']);
	});

	it('the question is announced politely', async () => {
		await mounted();
		drop('ABC-4', 'in_progress');
		const bar = await vi.waitFor(() => {
			flushSync();
			expect(nudge('ABC-4')).not.toBeNull();
			return nudge('ABC-4')!;
		});
		expect(bar.querySelector('[aria-live="polite"]')?.textContent).toBe('Serving which goal?');
	});

	it('a reload where the story now serves a goal (linked elsewhere) takes the bar away', async () => {
		await mounted();
		drop('ABC-4', 'in_progress');
		await vi.waitFor(() => {
			flushSync();
			expect(nudge('ABC-4')).not.toBeNull();
		});
		stories = stories.map((s) => (s.story.key === 'ABC-4' ? { ...s, story: { ...s.story, goals: [ref(1)] } } : s));
		for (const f of live.fns) f();
		await vi.waitFor(() => {
			flushSync();
			expect(nudge('ABC-4')).toBeNull();
		});
	});

	it('a reload where the story left active work takes the bar away', async () => {
		await mounted();
		drop('ABC-4', 'in_progress');
		await vi.waitFor(() => {
			flushSync();
			expect(nudge('ABC-4')).not.toBeNull();
		});
		stories = stories.map((s) => (s.story.key === 'ABC-4' ? { ...s, story: { ...s.story, delivery: 'todo' } } : s));
		for (const f of live.fns) f();
		await vi.waitFor(() => {
			flushSync();
			expect(q('[data-testid="column-todo"] [data-testid="card-ABC-4"]')).not.toBeNull();
		});
		expect(nudge('ABC-4')).toBeNull();
	});

	it('shows no bar when the reply says the moved story serves a goal (its epic links it), whatever the card said', async () => {
		replyGoals = [{ id: 1, outcome: 'Ship the import', via: 'epic' }];
		await mounted();
		drop('ABC-4', 'in_progress');
		await vi.waitFor(() => {
			flushSync();
			expect(q('[data-testid="column-in_progress"] [data-testid="card-ABC-4"]')).not.toBeNull();
		});
		expect(nudge('ABC-4')).toBeNull();
	});

	it('shows no bar with no active sprint', async () => {
		sprints = { active: null, closed: [], draft: null };
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(q('[data-testid="card-ABC-4"]')).not.toBeNull();
		});
		drop('ABC-4', 'in_progress');
		await vi.waitFor(() => {
			flushSync();
			expect(q('[data-testid="column-in_progress"] [data-testid="card-ABC-4"]')).not.toBeNull();
		});
		expect(nudge('ABC-4')).toBeNull();
	});

	it('when the move needs an override, the prompt comes first and the bar only after the overridden move lands', async () => {
		needsOverride = true;
		await mounted();
		drop('ABC-4', 'in_progress');
		const dialog = await vi.waitFor(() => {
			flushSync();
			const d = q('dialog');
			expect(d?.querySelector('h2')?.textContent).toBe('ABC-4 is not ready for in progress');
			return d!;
		});
		expect(nudge('ABC-4')).toBeNull();
		const why = dialog.querySelector('textarea')!;
		why.value = 'Urgent';
		why.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		buttonIn(dialog, 'Move anyway').click();
		await vi.waitFor(() => {
			flushSync();
			expect(q('dialog')).toBeNull();
			expect(q('[data-testid="column-in_progress"] li[data-key="ABC-4"] [data-testid="nudge"]')).not.toBeNull();
		});
		expect(net.writes.map((w) => w.body)).toEqual([
			{ version: 1, to: 'in_progress' },
			{ version: 1, to: 'in_progress', override_reason: 'Urgent' }
		]);
	});
});
