import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const get = vi.fn();
vi.mock('#lib/api/client.ts', () => ({ api: { GET: (path: string) => get(path) }, unwrap: (p: Promise<unknown>) => p }));
const onStreamMock = vi.fn();
vi.mock('#lib/events.svelte.ts', () => ({ onStream: (h: unknown) => onStreamMock(h) }));
const { load, refresh, followAccess, session } = await import('./me.svelte.ts');

const me = { email: 'dan@example.com', role: 'admin', token_kind: 'interactive' };
const config = { horizon_buckets: ['now'] };

beforeEach(() => get.mockReset());

describe('load', () => {
	it('retries after a failure instead of caching it, then shares the success', async () => {
		get.mockImplementation(async (path: string) => (path === '/api/v1/me' ? me : config));
		get.mockRejectedValueOnce(new Error('boom'));
		await expect(load()).rejects.toThrow('boom');
		await load();
		expect(session.me).toEqual(me);
		expect(session.config).toEqual(config);
		const calls = get.mock.calls.length;
		await load();
		expect(get.mock.calls.length).toBe(calls);
	});
});

describe('refresh', () => {
	it('replaces session.me, and an older answer never overwrites a newer one', async () => {
		let release: (v: unknown) => void = () => {};
		get.mockImplementationOnce(() => new Promise((r) => (release = r)));
		get.mockImplementationOnce(async () => ({ ...me, role: 'member' }));
		const first = refresh();
		await refresh();
		release({ ...me, role: 'admin' });
		await first;
		expect(session.me?.role).toBe('member');
	});
});

describe('followAccess', () => {
	it('refreshes on access and admin entries, whoever the subject, and on reopen, and ignores other entries', async () => {
		get.mockImplementation(async () => me);
		const hooks: { entry?: (e: { action: string }) => void; reopened?: () => void } = {};
		onStreamMock.mockImplementation((h: typeof hooks) => (Object.assign(hooks, h), () => {}));
		followAccess();
		for (const action of ['access_allowed', 'access_removed', 'admin_granted', 'admin_revoked']) hooks.entry!({ action });
		hooks.entry!({ action: 'moved' });
		hooks.reopened!();
		await Promise.resolve();
		expect(get.mock.calls.filter(([p]) => p === '/api/v1/me')).toHaveLength(6); // wiring + four access entries + reopen
	});
});

describe('followAccess retry', () => {
	const meCalls = () => get.mock.calls.filter(([p]) => p === '/api/v1/me').length;
	const wire = () => {
		const hooks: { entry?: (e: { action: string }) => void; reopened?: () => void } = {};
		onStreamMock.mockImplementation((h: typeof hooks) => (Object.assign(hooks, h), () => {}));
		return { hooks, stop: followAccess() };
	};
	beforeEach(() => vi.useFakeTimers());
	afterEach(() => vi.useRealTimers());

	it('retries a failure once after 5 s and schedules nothing after a second failure', async () => {
		get.mockRejectedValueOnce(new Error('down')).mockRejectedValueOnce(new Error('down'));
		const { stop } = wire();
		await vi.advanceTimersByTimeAsync(4999);
		expect(meCalls()).toBe(1);
		await vi.advanceTimersByTimeAsync(1);
		expect(meCalls()).toBe(2);
		await vi.advanceTimersByTimeAsync(60_000);
		expect(meCalls()).toBe(2);
		stop();
	});

	it('cancels a pending retry when a newer refresh runs', async () => {
		get.mockRejectedValueOnce(new Error('down'));
		get.mockImplementation(async () => me);
		const { hooks, stop } = wire();
		await vi.advanceTimersByTimeAsync(1000);
		hooks.reopened!();
		await vi.advanceTimersByTimeAsync(60_000);
		expect(meCalls()).toBe(2);
		stop();
	});
});
