// @vitest-environment jsdom
// The goal picker's store: which goals can be picked, the linking mode's goal, sequenced reloads and one link write at
// a time.
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Sprints } from '#lib/api/types.ts';
import { goal, sprint, sprintView } from '#lib/test/fixtures.ts';
import { failure } from '#lib/test/net.ts';
import { findGoal, linkingGoal, loadSprints, pickable, picker, resetPicker, startLinking, stopLinking, toggleLink } from './goalPicker.svelte.ts';

type Call = { method: string; path: string; body: unknown };
type Reply = { value: unknown; wait?: Promise<void> };
const net = vi.hoisted(() => ({ calls: [] as Call[], gets: [] as Reply[], sprints: null as unknown, fail: false, failGet: false, failWritesOnly: false }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	const { goal } = await import('#lib/test/fixtures.ts');
	installFetch(async ({ method, path, body }) => {
		net.calls.push({ method, path, body });
		if ((net.fail && (!net.failWritesOnly || method !== 'GET')) || (net.failGet && method === 'GET')) return failure('boom');
		if (method === 'GET' && path === '/api/v1/sprints') {
			// Queued replies first (each may wait), then the current sprints.
			const r = net.gets.shift();
			if (r?.wait) await r.wait;
			return r ? r.value : net.sprints;
		}
		if (/^\/api\/v1\/goals\/\d+\/links(\/[^/]+)?$/.test(path) && (method === 'POST' || method === 'DELETE')) return goal();
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

const draftGoal = goal({ id: 5, sprint_id: 2, outcome: 'Ship X' });
const activeGoal = goal({ id: 1, sprint_id: 1, outcome: 'Cut Y', links: ['ABC-6'] });
const all = (links: string[] = ['ABC-6']): Sprints => ({
	draft: sprint({ id: 2, state: 'draft', opened_at: null, goals: [draftGoal] }),
	active: sprintView(sprint({ goals: [{ ...activeGoal, links }] })),
	closed: [sprint({ id: 7, state: 'closed', closed_at: '2026-11-01T00:00:00Z', goals: [goal({ id: 9, sprint_id: 7 })] })],
	today: '2026-11-12'
});
const writes = () => net.calls.filter((c) => c.method !== 'GET');

afterEach(() => {
	resetPicker();
	net.calls.length = 0;
	net.gets.length = 0;
	net.sprints = null;
	net.fail = false;
	net.failGet = false;
	net.failWritesOnly = false;
});

describe('pickable and findGoal', () => {
	it('offers the draft, then the active sprint, and never a closed one', () => {
		expect(pickable(all()).map((g) => [g.label, g.sprint.id])).toEqual([
			['Next sprint', 2],
			['Active sprint', 1]
		]);
		expect(pickable({ ...all(), active: null }).map((g) => g.label)).toEqual(['Next sprint']);
		expect(pickable({ ...all(), draft: null }).map((g) => g.label)).toEqual(['Active sprint']);
		expect(pickable({ draft: null, active: null, closed: all().closed, today: '2026-11-12' })).toEqual([]);
		expect(pickable(null)).toEqual([]);
	});
	it('finds draft and active goals, never a closed sprint’s', () => {
		expect(findGoal(all(), 5)?.outcome).toBe('Ship X');
		expect(findGoal(all(), 1)?.outcome).toBe('Cut Y');
		expect(findGoal(all(), 9)).toBeNull();
		expect(findGoal(all(), null)).toBeNull();
	});
});

describe('loadSprints', () => {
	it('keeps the mode while the goal is pickable, and ends it when the goal is gone', async () => {
		net.sprints = all();
		startLinking(5);
		await loadSprints();
		expect(linkingGoal()?.outcome).toBe('Ship X');
		net.sprints = { ...all(), draft: null };
		await loadSprints();
		expect(picker.goalId).toBeNull();
	});
	it('drops a reload that answers after a newer one', async () => {
		let release = () => {};
		const wait = new Promise<void>((r) => (release = r));
		net.gets.push({ value: all(['OLD-1']), wait }, { value: all(['NEW-1']) });
		const first = loadSprints();
		await loadSprints();
		release();
		await first;
		expect(findGoal(picker.sprints, 1)?.links).toEqual(['NEW-1']);
	});
	it('clears an earlier failure once a load succeeds', async () => {
		net.fail = true;
		await loadSprints();
		expect(picker.error).toBe('boom');
		net.fail = false;
		net.sprints = all();
		await loadSprints();
		expect(picker.error).toBeNull();
	});
	it('drops the failure of a load that answers after a newer success', async () => {
		let release = () => {};
		const wait = new Promise<void>((r) => (release = r));
		net.gets.push({ value: failure('old'), wait }, { value: all() });
		const first = loadSprints();
		await loadSprints();
		release();
		await first;
		expect(picker.error).toBeNull();
		expect(picker.sprints).toEqual(all());
	});
	it('says why a load failed and keeps what it had', async () => {
		picker.sprints = all();
		net.fail = true;
		await loadSprints();
		expect(picker.error).toBe('boom');
		expect(picker.sprints).toEqual(all());
	});
});

describe('toggleLink', () => {
	it('links an unlinked key, then reloads', async () => {
		picker.sprints = all();
		net.sprints = all(['ABC-6', 'ABC-7']);
		startLinking(1);
		await toggleLink('ABC-7');
		expect(net.calls.map((c) => [c.method, c.path, c.body])).toEqual([
			['POST', '/api/v1/goals/1/links', { items: ['ABC-7'] }],
			['GET', '/api/v1/sprints', null]
		]);
		expect(linkingGoal()?.links).toEqual(['ABC-6', 'ABC-7']);
		expect(picker.writing).toBe(false);
	});
	it('unlinks a linked key', async () => {
		picker.sprints = all();
		net.sprints = all([]);
		startLinking(1);
		await toggleLink('ABC-6');
		expect(writes()).toEqual([{ method: 'DELETE', path: '/api/v1/goals/1/links/ABC-6', body: null }]);
		expect(linkingGoal()?.links).toEqual([]);
	});
	it('runs one write at a time', async () => {
		picker.sprints = all();
		net.sprints = all();
		startLinking(1);
		const first = toggleLink('ABC-7');
		await toggleLink('ABC-8'); // ignored: a write is in flight
		await first;
		expect(writes().map((c) => c.path)).toEqual(['/api/v1/goals/1/links']);
	});
	it('keeps the links and says why when the write fails', async () => {
		picker.sprints = all();
		startLinking(1);
		net.fail = true;
		await toggleLink('ABC-7');
		expect(picker.error).toBe('boom');
		expect(linkingGoal()?.links).toEqual(['ABC-6']);
		expect(picker.writing).toBe(false);
	});
	it('ends the mode quietly when the write fails and the reload shows the goal gone', async () => {
		picker.sprints = all();
		startLinking(1);
		net.fail = true;
		net.gets.push({ value: { draft: null, active: null, closed: [] } });
		// The write fails with `boom`; the reload (not failing) is what shows the goal gone.
		net.failWritesOnly = true;
		await toggleLink('ABC-7');
		expect(picker.goalId).toBeNull();
		expect(picker.error).toBeNull();
		expect(picker.writing).toBe(false);
	});
	it('reports a failed reload after a successful write and keeps the links', async () => {
		picker.sprints = all();
		net.failGet = true;
		startLinking(1);
		await toggleLink('ABC-7');
		expect(writes().map((c) => c.method)).toEqual(['POST']);
		expect(picker.error).toBe('boom');
		expect(picker.writing).toBe(false);
		expect(linkingGoal()?.links).toEqual(['ABC-6']);
	});
	it('writes nothing once linking stops', async () => {
		picker.sprints = all();
		startLinking(1);
		stopLinking();
		await toggleLink('ABC-7');
		expect(net.calls).toEqual([]);
	});
});
