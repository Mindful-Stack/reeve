// @vitest-environment jsdom
// The sprint page: the active sprint with the membership rule above its counts, confidence recorded by a
// person (a suggestion pre-fills the form, never shows as recorded), the draft's goals and links, the close step that
// ends in "Open next sprint", and the history, where a closed sprint shows its counts at close beside the derived ones
// only when they differ (kb/ADR-0014 R6).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ActivityEntry, Goal, Sprint, SprintCounts, Sprints, SprintView } from '#lib/api/types.ts';
import { activity, goal, sprint, sprintView } from '#lib/test/fixtures.ts';
import { RULE } from '#lib/sprint.ts';
import Page from './+page.svelte';

vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	return { page: fakePage({ url: '/sprint' }).page };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: () => {},
	everyVisible: () => {}
}));

type Write = { method: string; path: string; search: string; body: unknown };
const net = vi.hoisted(() => ({
	sprints: null as null | Sprints,
	views: {} as Record<number, SprintView>,
	closedEntries: {} as Record<number, ActivityEntry[]>,
	writes: [] as Write[],
	reads: [] as string[],
	/** Answers a write instead of the default (a value, or a Response for a refusal or error). */
	answer: null as null | ((w: Write) => unknown),
	/** Holds a read until the returned promise settles (to resolve reads out of order). */
	gate: null as null | ((path: string) => Promise<void> | undefined)
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(async ({ method, url, path, body }) => {
		if (method === 'GET') {
			net.reads.push(path + url.search);
			// The answer is what the server had when asked; a gate only delays its arrival.
			const one = path.match(/^\/api\/v1\/sprints\/(\d+)$/);
			let value: unknown;
			if (path === '/api/v1/sprints') value = structuredClone(net.sprints);
			else if (one) value = net.views[Number(one[1])];
			else if (path === '/api/v1/activity') value = net.closedEntries[Number(url.searchParams.get('entity_key'))] ?? [];
			else throw new Error(`unexpected GET ${path}`);
			const held = net.gate?.(path);
			if (held) await held;
			return value;
		}
		const w = { method, path, search: url.search, body };
		net.writes.push(w);
		if (net.answer) return net.answer(w);
		if (path === '/api/v1/goals') return { goal: goal({ id: 99 }), warnings: [] };
		if (path.endsWith('/close') || path.endsWith('/open')) return sprint();
		if (path.endsWith('/confidence')) return goal();
		return {};
	});
	return orig();
});

const refused = (reasons: unknown[]) => new Response(JSON.stringify({ outcome: 'refused', reasons }), { status: 200 });

let app: ReturnType<typeof mount> | null = null;
beforeEach(() => {
	net.sprints = { active: null, draft: null, closed: [], today: '2026-11-12' };
});
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.views = {};
	net.closedEntries = {};
	net.writes.length = 0;
	net.reads.length = 0;
	net.answer = null;
	net.gate = null;
});

async function open() {
	app = mount(Page, { target: document.body });
	await settle(() => expect(document.body.textContent).not.toContain('Loading…'));
}
async function settle(check: () => void) {
	await vi.waitFor(() => {
		flushSync();
		check();
	});
}
const text = (el: Element | null | undefined) => el?.textContent?.replace(/\s+/g, ' ').trim() ?? '';
const q = <T extends Element = HTMLElement>(sel: string, root: ParentNode = document.body) => root.querySelector(sel) as T | null;
const button = (name: string | RegExp, root: ParentNode = document.body) =>
	[...root.querySelectorAll('button')].find((b) => {
		const n = b.getAttribute('aria-label') ?? text(b);
		return typeof name === 'string' ? n === name : name.test(n);
	}) as
		| HTMLButtonElement
		| undefined;
function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
	el.value = value;
	el.dispatchEvent(new Event('input', { bubbles: true }));
}
function pick(radio: HTMLInputElement) {
	radio.checked = true;
	radio.dispatchEvent(new Event('change', { bubbles: true }));
	radio.dispatchEvent(new Event('click', { bubbles: true }));
	flushSync();
}
const writes = (path: string) => net.writes.filter((w) => w.path === path);

function counts(goals: SprintCounts['goals'], completed = 0, upkeep = 0): SprintCounts {
	return { goals, completed, upkeep_completed: upkeep };
}
const progress = (goal_id: number, over: Partial<SprintCounts['goals'][number]> = {}) => ({
	goal_id,
	completed: 0,
	in_flight: 0,
	todo: 0,
	in_review: 0,
	blocked_decisions: 0,
	...over
});

function activeWith(goals: Goal[], view: Partial<SprintView> = {}): SprintView {
	return sprintView(sprint({ id: 1, goals, version: 3 }), view);
}

describe('the active sprint', () => {
	it('links "blocked by n decisions" to the decisions page, which leads with records to review', async () => {
		// Records to review leave the this-sprint lane, so the link goes to the page, not to #this-sprint.
		net.sprints!.active = activeWith([goal({ id: 1 }), goal({ id: 2 })], {
			counts: counts([progress(1, { blocked_decisions: 2 }), progress(2, { blocked_decisions: 0 })])
		});
		await open();
		const link = q<HTMLAnchorElement>('[data-testid="goal-1"] a[href="/decisions"]');
		expect(text(link)).toBe('blocked by 2 decisions');
		expect(q('[data-testid="goal-2"] a')).toBeNull();
		expect(text(q('[data-testid="goal-2"] .counts'))).toContain('blocked by 0 decisions');
	});

	it('states the rule above the counts and shows each goal with its numbers, the upkeep line and the demo list', async () => {
		const g = goal({
			id: 1,
			outcome: 'Ana can book a room',
			check: 'one real booking',
			owner: 'ana@example.com',
			notes: 'pairs with the calendar work',
			confidence: 'at_risk',
			confidence_note: 'calendar API slow',
			links: ['RV-4']
		});
		net.sprints!.active = activeWith([g], {
			counts: counts([progress(1, { completed: 2, in_flight: 3, in_review: 1, todo: 4, blocked_decisions: 2 })], 24, 7),
			demo: [{ key: 'RV-9', title: 'Booking form' }]
		});
		await open();
		const panel = q('[data-testid="active-sprint"]')!;
		expect(text(panel)).toContain('2026-11-02 – 2026-11-13');
		const rule = [...panel.querySelectorAll('p')].find((p) => text(p) === RULE)!;
		const card = q('[data-testid="goal-1"]', panel)!;
		// The rule line is above the counts (kb/ADR-0014 R2).
		expect(rule.compareDocumentPosition(card) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
		const t = text(card);
		for (const s of ['Ana can book a room', 'one real booking', 'ana@example.com', 'pairs with the calendar work', 'calendar API slow'])
			expect(t).toContain(s);
		expect(text(q('[data-testid="confidence"]', card))).toBe('at risk');
		expect(t).toContain('2 completed');
		expect(t).toContain('3 in flight (1 in review)');
		expect(t).toContain('4 todo');
		expect(t).toContain('blocked by 2 decisions');
		expect(text(panel)).toContain('7 of 24 completed stories were upkeep (29%; team line 20%)');
		const demo = q('[data-testid="demo"]', panel)!;
		expect(text(demo)).toContain('done in the last 7 days');
		expect(text(demo)).toContain('RV-9');
		expect(text(demo)).toContain('Booking form');
	});

	it('shows a pending suggestion as a suggestion, never as the recorded confidence', async () => {
		const g = goal({ id: 1, suggested_confidence: 'off_track', suggested_by: 'agent@example.com', suggested_note: 'no PRs' });
		net.sprints!.active = activeWith([g]);
		await open();
		const card = q('[data-testid="goal-1"]')!;
		expect(text(q('[data-testid="confidence"]', card))).toBe('not recorded');
		expect(text(q('[data-testid="suggestion"]', card))).toContain('suggested: off track by agent@example.com');
	});
});

describe('recording confidence', () => {
	it('opens a form of three options and a note, and posts the choice', async () => {
		net.sprints!.active = activeWith([goal({ id: 1 })]);
		await open();
		button('Record confidence')!.click();
		flushSync();
		const form = q<HTMLFormElement>('form[aria-label="Record confidence"]')!;
		const radios = [...form.querySelectorAll<HTMLInputElement>('input[type="radio"]')];
		expect(radios.map((r) => r.value)).toEqual(['on_track', 'at_risk', 'off_track']);
		expect(radios.some((r) => r.checked)).toBe(false);
		pick(radios[1]);
		type(q<HTMLTextAreaElement>('textarea', form)!, 'slow API');
		flushSync();
		button('Record', form)!.click();
		await settle(() => expect(writes('/api/v1/goals/1/confidence')).toHaveLength(1));
		expect(writes('/api/v1/goals/1/confidence')[0].body).toEqual({ confidence: 'at_risk', note: 'slow API' });
	});

	it('opens pre-filled with a suggestion, which can be recorded as is', async () => {
		const g = goal({ id: 1, suggested_confidence: 'off_track', suggested_by: 'agent@example.com', suggested_note: 'no PRs' });
		net.sprints!.active = activeWith([g]);
		await open();
		button('Record confidence')!.click();
		flushSync();
		const form = q<HTMLFormElement>('form[aria-label="Record confidence"]')!;
		expect(text(form)).toContain('Suggested by agent@example.com: no PRs');
		expect(q<HTMLInputElement>('input[value="off_track"]', form)!.checked).toBe(true);
		expect(q<HTMLTextAreaElement>('textarea', form)!.value).toBe('no PRs');
		button('Record', form)!.click();
		await settle(() => expect(writes('/api/v1/goals/1/confidence')).toHaveLength(1));
		expect(writes('/api/v1/goals/1/confidence')[0].body).toEqual({ confidence: 'off_track', note: 'no PRs' });
	});

	it('can change a suggestion before recording, or be dismissed without posting', async () => {
		const g = goal({ id: 1, suggested_confidence: 'off_track', suggested_by: 'agent@example.com', suggested_note: 'no PRs' });
		net.sprints!.active = activeWith([g]);
		await open();
		button('Record confidence')!.click();
		flushSync();
		button('Cancel', q('form[aria-label="Record confidence"]')!)!.click();
		flushSync();
		expect(q('form[aria-label="Record confidence"]')).toBeNull();
		expect(net.writes).toHaveLength(0);

		button('Record confidence')!.click();
		flushSync();
		const form = q<HTMLFormElement>('form[aria-label="Record confidence"]')!;
		pick(q<HTMLInputElement>('input[value="on_track"]', form)!);
		button('Record', form)!.click();
		await settle(() => expect(writes('/api/v1/goals/1/confidence')).toHaveLength(1));
		expect(writes('/api/v1/goals/1/confidence')[0].body).toMatchObject({ confidence: 'on_track' });
	});
});

describe('the next sprint (draft)', () => {
	const draft = (goals: Goal[] = []): Sprint => sprint({ id: 2, state: 'draft', starts_on: '2026-11-16', ends_on: '2026-11-27', opened_at: null, goals, version: 5 });

	it('adds a goal with a fresh idempotency key, and warns past three goals', async () => {
		net.sprints!.draft = draft();
		net.answer = (w) => (w.path === '/api/v1/goals' ? { goal: goal({ id: 9, sprint_id: 2 }), warnings: ['more_than_three_goals'] } : {});
		await open();
		const form = q<HTMLFormElement>('form[aria-label="New goal"]')!;
		const outcome = q<HTMLInputElement>('input[name="outcome"]', form)!;
		expect(outcome.placeholder).toBe('By 2026-11-27, <who> can <outcome>');
		type(outcome, 'By Friday, Ana can book a room');
		type(q<HTMLInputElement>('input[name="check"]', form)!, 'one real booking');
		type(q<HTMLInputElement>('input[name="owner"]', form)!, 'ana@example.com');
		type(q<HTMLTextAreaElement>('textarea[name="notes"]', form)!, 'pairs');
		flushSync();
		button('Add goal', form)!.click();
		await settle(() => expect(writes('/api/v1/goals')).toHaveLength(1));
		const body = writes('/api/v1/goals')[0].body as Record<string, unknown>;
		expect(body).toMatchObject({ outcome: 'By Friday, Ana can book a room', check: 'one real booking', owner: 'ana@example.com', notes: 'pairs' });
		expect(body.idempotency_key).toMatch(/^[0-9a-f-]{36}$/);
		expect(body.active).toBeUndefined();
		await settle(() => expect(text(q('[data-testid="next-sprint"]'))).toContain('more than three goals thins focus'));

		// The next goal is another goal, so it carries another key.
		type(q<HTMLInputElement>('input[name="outcome"]', form)!, 'another');
		type(q<HTMLInputElement>('input[name="check"]', form)!, 'a check');
		flushSync();
		button('Add goal', form)!.click();
		await settle(() => expect(writes('/api/v1/goals')).toHaveLength(2));
		expect((writes('/api/v1/goals')[1].body as Record<string, unknown>).idempotency_key).not.toBe(body.idempotency_key);
	});

	it('shows no warning when the answer has none', async () => {
		net.sprints!.draft = draft();
		await open();
		const form = q<HTMLFormElement>('form[aria-label="New goal"]')!;
		type(q<HTMLInputElement>('input[name="outcome"]', form)!, 'x');
		type(q<HTMLInputElement>('input[name="check"]', form)!, 'y');
		flushSync();
		button('Add goal', form)!.click();
		await settle(() => expect(writes('/api/v1/goals')).toHaveLength(1));
		expect(text(q('[data-testid="next-sprint"]'))).not.toContain('thins focus');
	});

	it('removes a goal at its version, and links and unlinks an epic or initiative by key', async () => {
		net.sprints!.draft = draft([goal({ id: 5, sprint_id: 2, version: 2, links: ['RV-3'] })]);
		await open();
		const card = q('[data-testid="goal-5"]')!;
		const key = q<HTMLInputElement>('input[name="link"]', card)!;
		type(key, ' RV-7 ');
		flushSync();
		button('Link', card)!.click();
		await settle(() => expect(writes('/api/v1/goals/5/links')).toHaveLength(1));
		expect(writes('/api/v1/goals/5/links')[0].body).toEqual({ items: ['RV-7'] });

		button('Unlink RV-3', card)!.click();
		await settle(() => expect(writes('/api/v1/goals/5/links/RV-3')).toHaveLength(1));
		expect(writes('/api/v1/goals/5/links/RV-3')[0].method).toBe('DELETE');

		button('Remove', card)!.click();
		await settle(() => expect(writes('/api/v1/goals/5')).toHaveLength(1));
		expect(writes('/api/v1/goals/5')[0]).toMatchObject({ method: 'DELETE', search: '?version=2' });
	});

	it('shows a refusal in words', async () => {
		net.sprints!.draft = draft([goal({ id: 5, sprint_id: 2 })]);
		net.answer = () => refused([{ reason: 'sprint_not_draft' }]);
		await open();
		button('Remove', q('[data-testid="goal-5"]')!)!.click();
		await settle(() => expect(text(document.body)).toContain("only a draft sprint's goals can be removed"));
	});
});

describe('editing a goal', () => {
	async function edit(where: 'active' | 'draft') {
		const g = goal({ id: 5, sprint_id: where === 'active' ? 1 : 2, outcome: 'Old', check: 'old check', owner: 'ana@example.com', notes: 'n', version: 4 });
		if (where === 'active') net.sprints!.active = activeWith([g]);
		else net.sprints!.draft = sprint({ id: 2, state: 'draft', opened_at: null, goals: [g] });
		await open();
		const card = q(`[data-testid="${where === 'active' ? 'active-sprint' : 'next-sprint'}"] [data-testid="goal-5"]`)!;
		button('Edit', card)!.click();
		flushSync();
		const form = q<HTMLFormElement>('form[aria-label="Edit goal"]', card)!;
		expect(q<HTMLInputElement>('input[name="outcome"]', form)!.value).toBe('Old');
		expect(q<HTMLInputElement>('input[name="check"]', form)!.value).toBe('old check');
		expect(q<HTMLInputElement>('input[name="owner"]', form)!.value).toBe('ana@example.com');
		expect(q<HTMLTextAreaElement>('textarea[name="notes"]', form)!.value).toBe('n');
		type(q<HTMLInputElement>('input[name="outcome"]', form)!, 'New');
		type(q<HTMLTextAreaElement>('textarea[name="notes"]', form)!, '');
		flushSync();
		button('Save', form)!.click();
		return card;
	}

	it.each(['active', 'draft'] as const)('edits a %s goal at its version', async (where) => {
		await edit(where);
		await settle(() => expect(writes('/api/v1/goals/5')).toHaveLength(1));
		expect(writes('/api/v1/goals/5')[0]).toMatchObject({
			method: 'PATCH',
			body: { version: 4, outcome: 'New', check: 'old check', owner: 'ana@example.com', notes: '' }
		});
		await settle(() => expect(q('form[aria-label="Edit goal"]')).toBeNull());
	});

	it('on a conflict keeps the typed form, refetches, says so, and saves again at the current version', async () => {
		let first = true;
		net.answer = (w) => {
			if (w.method !== 'PATCH') return {};
			if (!first) return goal({ id: 5, version: 6 });
			first = false;
			// Someone else saved meanwhile: the refetch serves the goal at version 5.
			net.sprints!.active = activeWith([goal({ id: 5, outcome: 'Theirs', check: 'old check', version: 5 })]);
			return new Response(JSON.stringify({ outcome: 'error', code: 'conflict', message: 'stale', current: 5 }), { status: 409 });
		};
		await edit('active');
		await settle(() => expect(text(document.body)).toContain('someone else changed this goal; reloaded'));
		await settle(() => expect(text(q('[data-testid="active-sprint"] h3'))).toBe('Theirs'));
		const form = q<HTMLFormElement>('form[aria-label="Edit goal"]')!;
		expect(form).not.toBeNull();
		expect(q<HTMLInputElement>('input[name="outcome"]', form)!.value).toBe('New');
		button('Save', form)!.click();
		await settle(() => expect(writes('/api/v1/goals/5')).toHaveLength(2));
		expect(writes('/api/v1/goals/5')[1].body).toMatchObject({ version: 5, outcome: 'New' });
		await settle(() => expect(q('form[aria-label="Edit goal"]')).toBeNull());
	});
});

describe('links on an active goal', () => {
	it('links and unlinks an epic or initiative by key, as on the draft', async () => {
		net.sprints!.active = activeWith([goal({ id: 1, links: ['RV-3'] })]);
		await open();
		const card = q('[data-testid="active-sprint"] [data-testid="goal-1"]')!;
		type(q<HTMLInputElement>('input[name="link"]', card)!, 'RV-8');
		flushSync();
		button('Link', card)!.click();
		await settle(() => expect(writes('/api/v1/goals/1/links')).toHaveLength(1));
		expect(writes('/api/v1/goals/1/links')[0].body).toEqual({ items: ['RV-8'] });
		button('Unlink RV-3', card)!.click();
		await settle(() => expect(writes('/api/v1/goals/1/links/RV-3')).toHaveLength(1));
		expect(writes('/api/v1/goals/1/links/RV-3')[0].method).toBe('DELETE');
	});
});

describe('the close step', () => {
	const goals = () => [
		goal({ id: 1, outcome: 'Book a room', check: 'one booking', confidence: 'on_track' }),
		goal({ id: 2, outcome: 'Export', check: 'a CSV', confidence: null })
	];
	async function openClose() {
		net.sprints!.active = activeWith(goals(), { counts: counts([progress(1, { completed: 3 }), progress(2, { completed: 0 })], 3) });
		net.sprints!.draft = sprint({ id: 2, state: 'draft', opened_at: null, version: 8 });
		await open();
		button('Close…')!.click();
		flushSync();
		return q<HTMLElement>('[data-testid="close-sprint"]')!;
	}
	const verdict = (step: HTMLElement, id: number, v: string) => pick(q<HTMLInputElement>(`input[name="verdict-${id}"][value="${v}"]`, step)!);

	it('lists every goal with its check, confidence and completed count, and closes only with every verdict', async () => {
		const step = await openClose();
		const row1 = text(q('[data-testid="close-goal-1"]', step));
		expect(row1).toContain('one booking');
		expect(row1).toContain('on track');
		expect(row1).toContain('3 completed');
		const row2 = text(q('[data-testid="close-goal-2"]', step));
		expect(row2).toContain('a CSV');
		expect(row2).toContain('not recorded');
		const close = button('Close sprint', step)!;
		expect(close.disabled).toBe(true);
		verdict(step, 1, 'met');
		expect(close.disabled).toBe(true);
		verdict(step, 2, 'missed');
		expect(close.disabled).toBe(false);
		close.click();
		await settle(() => expect(writes('/api/v1/sprints/1/close')).toHaveLength(1));
		expect(writes('/api/v1/sprints/1/close')[0].body).toEqual({
			version: 3,
			verdicts: [
				{ goal_id: 1, verdict: 'met', note: null },
				{ goal_id: 2, verdict: 'missed', note: null }
			]
		});
	});

	it('marks the goals a verdict_missing refusal names', async () => {
		const step = await openClose();
		net.answer = () => refused([{ reason: 'verdict_missing', goals: [2] }]);
		verdict(step, 1, 'met');
		verdict(step, 2, 'partly_met');
		button('Close sprint', step)!.click();
		await settle(() => expect(q('[data-testid="close-goal-2"] [role="radiogroup"]', step)!.getAttribute('aria-invalid')).toBe('true'));
		expect(q('[data-testid="close-goal-1"] [role="radiogroup"]', step)!.getAttribute('aria-invalid')).toBeNull();
		expect(text(q('[data-testid="close-goal-2"]', step))).toContain('needs a verdict');
	});

	it('refetches the goals when verdict_missing names one the step does not list, and marks it', async () => {
		const step = await openClose();
		// A goal was added to the active sprint after the step opened.
		const g3 = goal({ id: 3, outcome: 'Late goal', check: 'c3' });
		net.views[1] = activeWith([...goals(), g3], { sprint: sprint({ id: 1, goals: [...goals(), g3], version: 4 }) });
		let refuse = true;
		net.answer = (w) => {
			if (!w.path.endsWith('/close')) return {};
			if (refuse) {
				refuse = false;
				return refused([{ reason: 'verdict_missing', goals: [3] }]);
			}
			return sprint();
		};
		verdict(step, 1, 'met');
		verdict(step, 2, 'missed');
		button('Close sprint', step)!.click();
		await settle(() => expect(q('[data-testid="close-goal-3"]', step)).not.toBeNull());
		expect(q('[data-testid="close-goal-3"] [role="radiogroup"]', step)!.getAttribute('aria-invalid')).toBe('true');
		expect(text(q('[data-testid="close-goal-3"]', step))).toContain('c3');
		expect(button('Close sprint', step)!.disabled).toBe(true);
		verdict(step, 3, 'partly_met');
		button('Close sprint', step)!.click();
		await settle(() => expect(writes('/api/v1/sprints/1/close')).toHaveLength(2));
		expect(writes('/api/v1/sprints/1/close')[1].body).toEqual({
			version: 4,
			verdicts: [
				{ goal_id: 1, verdict: 'met', note: null },
				{ goal_id: 2, verdict: 'missed', note: null },
				{ goal_id: 3, verdict: 'partly_met', note: null }
			]
		});
	});

	it('ends with "Open next sprint", which opens the draft', async () => {
		const step = await openClose();
		verdict(step, 1, 'met');
		verdict(step, 2, 'missed');
		button('Close sprint', step)!.click();
		await settle(() => expect(button('Open next sprint', step)).toBeDefined());
		button('Open next sprint', step)!.click();
		await settle(() => expect(writes('/api/v1/sprints/2/open')).toHaveLength(1));
		expect(writes('/api/v1/sprints/2/open')[0].body).toEqual({ version: 8 });
	});
});

describe('the history', () => {
	const closed = (id: number, goals: Goal[]) => sprint({ id, state: 'closed', closed_at: '2026-10-30T09:00:00Z', goals });
	function entry(id: number, c: SprintCounts): ActivityEntry {
		return activity({ entity_type: 'sprint', entity_key: String(id), action: 'closed', payload: { counts: c } });
	}
	async function expand(id: number) {
		await open();
		button(/^Show numbers/, q(`[data-testid="history-${id}"]`)!)!.click();
		await settle(() => expect(q(`[data-testid="history-${id}"] [data-testid="numbers"]`)).not.toBeNull());
		return q<HTMLElement>(`[data-testid="history-${id}"]`)!;
	}

	it('lists closed sprints in the order served (newest first) with goals, confidence and verdicts', async () => {
		net.sprints!.closed = [
			closed(7, [goal({ id: 3, outcome: 'Later', sprint_id: 7, confidence: 'on_track', verdict: 'partly_met', verdict_note: 'one booking' })]),
			closed(4, [goal({ id: 1, outcome: 'Earlier', sprint_id: 4, verdict: 'met' })])
		];
		await open();
		const rows = [...document.body.querySelectorAll('[data-testid^="history-"]')].map((r) => r.getAttribute('data-testid'));
		expect(rows).toEqual(['history-7', 'history-4']);
		const t = text(q('[data-testid="history-7"]'));
		expect(t).toContain('Later');
		expect(t).toContain('on track');
		expect(t).toContain('partly met');
		expect(t).toContain('one booking');
	});

	it('loads the view and the closed entry and shows "now · at close" where a number changed', async () => {
		const g = goal({ id: 3, sprint_id: 7 });
		net.sprints!.closed = [closed(7, [g])];
		net.views[7] = sprintView(closed(7, [g]), { counts: counts([progress(3, { completed: 4, todo: 1, blocked_decisions: 5 })], 6, 2) });
		net.closedEntries[7] = [entry(7, counts([progress(3, { completed: 3, todo: 1, blocked_decisions: 1 })], 5, 2))];
		const row = await expand(7);
		expect(net.reads).toContain('/api/v1/sprints/7');
		expect(net.reads).toContain('/api/v1/activity?entity_type=sprint&entity_key=7&action=closed');
		const numbers = q('[data-testid="numbers"]', row)!;
		expect(text(numbers)).toContain(RULE);
		expect(text(numbers)).toContain('changed since close: items moved between parents or reopened');
		const goalNumbers = text(q('[data-testid="numbers-3"]', numbers));
		expect(goalNumbers).toContain('now 4 · at close 3 completed');
		expect(goalNumbers).toContain('1 todo');
		expect(goalNumbers).not.toContain('now 1');
		// Blocked-by-decisions is the value at close only, never the recomputed one.
		expect(goalNumbers).toContain('blocked by 1 decision at close');
		expect(goalNumbers).not.toContain('5');
		expect(text(numbers)).toContain('2 of 6 completed stories were upkeep (at close: 2 of 5) · team line 20%');
	});

	// Equal counts: no now·at-close comparison and no changed-since-close note; blocked-by-decisions still shows, as the
	// closed entry's value labelled "at close" (it is never recomputed for a closed sprint).
	it('shows one set of numbers when nothing changed since close, with blocked-by-decisions at close only', async () => {
		const g = goal({ id: 3, sprint_id: 7 });
		net.sprints!.closed = [closed(7, [g])];
		const c = counts([progress(3, { completed: 3, blocked_decisions: 2 })], 5, 1);
		net.views[7] = sprintView(closed(7, [g]), { counts: { ...c, goals: [progress(3, { completed: 3, blocked_decisions: 0 })] } });
		net.closedEntries[7] = [entry(7, c)];
		const numbers = q('[data-testid="numbers"]', await expand(7))!;
		expect(text(numbers)).not.toContain('· at close');
		expect(text(numbers)).not.toContain('changed since close');
		expect(text(q('[data-testid="numbers-3"]', numbers))).toContain('3 completed');
		expect(text(q('[data-testid="numbers-3"]', numbers))).toContain('blocked by 2 decisions at close');
		expect(text(numbers)).toContain('1 of 5 completed stories were upkeep (20%; team line 20%)');
	});

	it('keeps the newest load when two loads of one entry resolve out of order', async () => {
		const g = goal({ id: 3, sprint_id: 7 });
		net.sprints!.closed = [closed(7, [g])];
		net.views[7] = sprintView(closed(7, [g]), { counts: counts([progress(3, { completed: 1 })], 1) });
		let release!: () => void;
		const first = new Promise<void>((r) => (release = r));
		let held = false;
		net.gate = (path) => {
			if (path !== '/api/v1/sprints/7' || held) return undefined;
			held = true;
			return first;
		};
		await open();
		const row = q('[data-testid="history-7"]')!;
		button(/^Show numbers/, row)!.click(); // load 1, held with the old numbers
		flushSync();
		button(/^Hide numbers/, row)!.click();
		flushSync();
		net.views[7] = sprintView(closed(7, [g]), { counts: counts([progress(3, { completed: 2 })], 2) });
		button(/^Show numbers/, row)!.click(); // load 2, answered at once
		await settle(() => expect(text(q('[data-testid="numbers-3"]', row))).toContain('2 completed'));
		release();
		await new Promise((r) => setTimeout(r, 20));
		flushSync();
		expect(text(q('[data-testid="numbers-3"]', row))).toContain('2 completed');
	});

	// A closed sprint always has its `closed` entry (the CLI treats a missing one as an error): without it the derived
	// numbers show under a one-line note, and no blocked-by value (the live one is today's, not the sprint's).
	it('says there is no record of the counts at close when the sprint has no closed entry', async () => {
		const g = goal({ id: 3, sprint_id: 7 });
		net.sprints!.closed = [closed(7, [g])];
		net.views[7] = sprintView(closed(7, [g]), { counts: counts([progress(3, { completed: 2, blocked_decisions: 4 })], 2) });
		const numbers = q('[data-testid="numbers"]', await expand(7))!;
		expect(text(q('[data-testid="no-close-record"]', numbers))).toBe('no record of the counts at close');
		expect(text(numbers)).not.toContain('· at close');
		expect(text(numbers)).not.toContain('blocked by');
		expect(text(q('[data-testid="numbers-3"]', numbers))).toContain('2 completed');
	});

	it('shows no such note when the closed entry is there', async () => {
		const g = goal({ id: 3, sprint_id: 7 });
		net.sprints!.closed = [closed(7, [g])];
		const c = counts([progress(3, { completed: 2 })], 2);
		net.views[7] = sprintView(closed(7, [g]), { counts: c });
		net.closedEntries[7] = [entry(7, c)];
		const numbers = q('[data-testid="numbers"]', await expand(7))!;
		expect(q('[data-testid="no-close-record"]', numbers)).toBeNull();
	});

	describe('its promises', () => {
		const promise = (key: string, over: Partial<SprintView['promises'][number]> = {}): SprintView['promises'][number] => ({
			key,
			title: `Title ${key}`,
			on: '2026-11-20',
			to: null,
			holder: 'own',
			epic: null,
			state: 'kept',
			done_on: '2026-11-19',
			days: null,
			...over
		});
		const dated = (id: number, starts_on: string, ends_on: string) => sprint({ id, state: 'closed', closed_at: `${ends_on}T16:00:00Z`, starts_on, ends_on });

		it('lists them under the dates after the previous sprint\'s planned end, then the note', async () => {
			net.sprints!.closed = [dated(7, '2026-11-11', '2026-11-24'), dated(6, '2026-10-28', '2026-11-10')];
			net.views[7] = sprintView(dated(7, '2026-11-11', '2026-11-24'), {
				promises: [
					promise('ABC-12', { to: 'Acme' }),
					promise('ABC-14', { on: '2026-11-21', state: 'late', done_on: '2026-11-23', days: 2 }),
					promise('ABC-15', { on: '2026-11-24', state: 'overdue', done_on: null, days: 3, holder: 'epic', epic: 'ABC-7' })
				]
			});
			const row = await expand(7);
			expect(button(/^Hide numbers and promises$/, row)).toBeDefined();
			const section = q('[data-testid="promises"]', row)!;
			expect(text(q('h4', section))).toBe('Promises due 11–24 Nov');
			const items = [...section.querySelectorAll('li')];
			expect(items.map(text)).toEqual([
				'ABC-12 Title ABC-12 · to Acme · Kept · 20 Nov',
				'ABC-14 Title ABC-14 · Done 2 d late · 21 Nov',
				'ABC-15 Title ABC-15 · Overdue 3 d · 24 Nov'
			]);
			expect(q('a', items[0])!.getAttribute('href')).toBe('?story=ABC-12');
			expect(text(q('a', items[0]))).toBe('ABC-12');
			expect(text(section)).toContain('A promise is to a date: work done after it counts as late, even inside the sprint.');
		});

		it('spans two months, and the first sprint takes every date up to its end', async () => {
			net.sprints!.closed = [dated(7, '2026-10-28', '2026-11-10'), dated(6, '2026-10-14', '2026-10-27')];
			net.views[7] = sprintView(dated(7, '2026-10-28', '2026-11-10'), { promises: [promise('ABC-12', { on: '2026-11-02' })] });
			net.views[6] = sprintView(dated(6, '2026-10-14', '2026-10-27'), { promises: [promise('ABC-9', { on: '2026-10-20' })] });
			expect(text(q('[data-testid="promises"] h4', await expand(7)))).toBe('Promises due 28 Oct–10 Nov');
			button(/^Show numbers/, q('[data-testid="history-6"]')!)!.click();
			await settle(() => expect(q('[data-testid="history-6"] [data-testid="promises"]')).not.toBeNull());
			expect(text(q('[data-testid="history-6"] [data-testid="promises"] h4'))).toBe('Promises due up to 27 Oct');
		});

		it('shows no section without promises', async () => {
			net.sprints!.closed = [dated(7, '2026-11-11', '2026-11-24')];
			net.views[7] = sprintView(dated(7, '2026-11-11', '2026-11-24'));
			const row = await expand(7);
			expect(q('[data-testid="promises"]', row)).toBeNull();
			expect(text(row)).not.toContain('A promise is to a date');
		});
	});
});
