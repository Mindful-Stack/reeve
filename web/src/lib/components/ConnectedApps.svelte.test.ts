// @vitest-environment jsdom
// "Connected apps": the list, what each row offers, what Revoke sends, and that it follows live updates and the clock.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Connection } from '#lib/api/types.ts';
import { connection } from '#lib/test/fixtures.ts';
import { settle } from '#lib/test/net.ts';

const liveFns = vi.hoisted(() => [] as (() => unknown)[]);
const polls = vi.hoisted(() => [] as { run: () => unknown; period: number }[]);
// The real minute clock; the live stream and the visible-tab timer are captured and run by hand.
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => void liveFns.push(fn),
	everyVisible: (run: () => unknown, period: number) => void polls.push({ run, period })
}));
type Deferred = { resolve: (rows: unknown[]) => void; reject: (e: Error) => void };
const net = vi.hoisted(() => ({
	calls: [] as { method: string; path: string; init: unknown }[],
	rows: [] as unknown[],
	fail: null as Error | null,
	failDelete: null as Error | null,
	/** When set, each GET waits here until the test settles it. */
	pending: null as Deferred[] | null
}));
vi.mock('#lib/api/client.ts', () => {
	const respond = (method: string) => async (path: string, init: unknown) => {
		net.calls.push({ method, path, init });
		if (net.fail) throw net.fail;
		if (method === 'DELETE' && net.failDelete) throw net.failDelete;
		if (method === 'GET' && net.pending) {
			const pending = net.pending;
			return new Promise((resolve, reject) => void pending.push({ resolve, reject }));
		}
		return method === 'GET' ? net.rows : {};
	};
	return {
		api: { GET: respond('GET'), DELETE: respond('DELETE') },
		unwrap: (p: Promise<unknown>) => p,
		ApiError: class extends Error {},
		Conflict: class extends Error {},
		Refused: class extends Error {}
	};
});
const { default: ConnectedApps } = await import('./ConnectedApps.svelte');

// A fixed "now" between the fixture's approval and its end: only `Date` is faked, timers run.
const NOW = new Date('2026-11-14T09:00:00Z');
let app: ReturnType<typeof mount> | null = null;
let asked: string[] = [];
let confirmed = true;
beforeEach(() => {
	vi.useFakeTimers({ now: NOW, toFake: ['Date'] });
	vi.stubGlobal('confirm', (text: string) => {
		asked.push(text);
		return confirmed;
	});
});
afterEach(() => {
	if (app) unmount(app);
	app = null;
	net.calls.length = 0;
	net.rows = [];
	net.fail = null;
	net.failDelete = null;
	net.pending = null;
	liveFns.length = 0;
	polls.length = 0;
	asked = [];
	confirmed = true;
	vi.useRealTimers();
	vi.unstubAllGlobals();
	document.body.innerHTML = '';
});

async function open(rows: Connection[], props: { owner?: string } = {}) {
	net.rows = rows;
	app = mount(ConnectedApps, { target: document.body, props });
	await vi.waitFor(() => expect(document.body.textContent).not.toContain('Loading'));
	flushSync();
}
const rowOf = (id: number) => document.querySelector(`li[data-connection="${id}"]`) as HTMLElement;
const revokeButton = (id: number) => rowOf(id).querySelector('button') as HTMLButtonElement | null;

describe('ConnectedApps', () => {
	// kb/ADR-0029 R9: the client's name, and what the design lists. Catches a row built from anything but the answer.
	it('shows the app, what it may do and its dates, and offers Revoke on a live connection', async () => {
		await open([connection({ refreshed_at: '2026-11-13T08:00:00Z' }), connection({ id: 8, scope: 'write', client_host: 'other.example', client_name: 'Other' })]);
		expect(net.calls).toEqual([{ method: 'GET', path: '/api/v1/connections', init: { params: { query: {} } } }]);
		const row = rowOf(7).textContent ?? '';
		for (const part of ['client.example', 'Example App', 'Read', 'Connected 2026-11-01', 'Ends 2026-12-01', 'Last refreshed 2026-11-13']) {
			expect(row, part).toContain(part);
		}
		expect(row).not.toContain('Read and change');
		expect(rowOf(8).textContent).toContain('Read and change');
		expect(rowOf(8).textContent).toContain('Last refreshed never');
		expect(revokeButton(7)?.textContent).toBe('Revoke');
	});

	it('asks for another person’s connections when given an owner', async () => {
		await open([], { owner: 'bob@example.com' });
		expect(net.calls[0]).toEqual({ method: 'GET', path: '/api/v1/connections', init: { params: { query: { owner: 'bob@example.com' } } } });
		expect(document.body.textContent).toContain('No connected apps.');
	});

	// Catches a Revoke that names the wrong connection, one that acts without the confirmation, a confirmation that does
	// not say whose app it is on the users page, and a stale list.
	it('Revoke asks first, naming the person when it is not the viewer, deletes that connection and reloads', async () => {
		await open([connection()], { owner: 'bob@example.com' });
		confirmed = false;
		revokeButton(7)!.click();
		flushSync();
		expect(asked).toEqual(['Revoke client.example for bob@example.com? A chat using it stops at its next step and has to be connected again.']);
		expect(net.calls.filter((c) => c.method === 'DELETE')).toHaveLength(0);
		confirmed = true;
		revokeButton(7)!.click();
		await vi.waitFor(() => expect(net.calls.filter((c) => c.method === 'GET')).toHaveLength(2));
		expect(net.calls.find((c) => c.method === 'DELETE')).toEqual({ method: 'DELETE', path: '/api/v1/connections/{id}', init: { params: { path: { id: 7 } } } });
	});

	it('asks without a name for the viewer’s own app', async () => {
		await open([connection()]);
		confirmed = false;
		revokeButton(7)!.click();
		expect(asked).toEqual(['Revoke client.example? A chat using it stops at its next step and has to be connected again.']);
	});

	// Every way a connection stops: its reason, no Revoke, and its dates still there.
	it.each([
		[{ ends_at: '2026-11-10T09:00:00Z' }, 'Ended 2026-11-10'],
		[{ revoked_at: '2026-11-12T09:00:00Z', revoked_reason: 'revoked' as const }, 'Revoked 2026-11-12'],
		[{ revoked_at: '2026-11-12T09:00:00Z', revoked_reason: 'refresh_token_reused' as const }, 'Ended 2026-11-12: the app’s sign-in went out of step; connect it again'],
		[{ revoked_at: '2026-11-12T09:00:00Z', revoked_reason: 'user_disabled' as const }, 'Ended 2026-11-12: the account was disabled'],
		[{ revoked_at: '2026-11-12T09:00:00Z', revoked_reason: 'restored' as const }, 'Ended 2026-11-12: the instance was restored from a backup']
	])('a stopped connection says why and cannot be revoked: %#', async (over, why) => {
		await open([connection({ refreshed_at: '2026-11-09T08:00:00Z', ...over })]);
		const row = rowOf(7);
		expect(row.textContent).toContain(why);
		expect(row.textContent).toContain('Connected 2026-11-01');
		expect(row.textContent).toContain('Last refreshed 2026-11-09');
		expect(row.classList.contains('stopped')).toBe(true);
		expect(revokeButton(7)).toBeNull();
	});

	// The existing live mechanism: a change elsewhere (a revoke on another device, a disable) reaches the open list.
	// Catches a list that only loads once.
	it('reloads on a live update and shows a connection that was revoked elsewhere as stopped', async () => {
		await open([connection()]);
		expect(revokeButton(7)).not.toBeNull();
		net.rows = [connection({ revoked_at: '2026-11-14T08:59:00Z', revoked_reason: 'user_disabled' })];
		expect(liveFns).toHaveLength(1);
		await liveFns[0]();
		flushSync();
		expect(rowOf(7).textContent).toContain('Ended 2026-11-14: the account was disabled');
		expect(revokeButton(7)).toBeNull();
	});

	// Catches a failed revoke that leaves the list as it was: the connection may have been revoked elsewhere meanwhile.
	it('a failed revoke says why and reloads the list', async () => {
		await open([connection()]);
		net.failDelete = new Error('gone');
		revokeButton(7)!.click();
		await vi.waitFor(() => expect(net.calls.filter((c) => c.method === 'GET')).toHaveLength(2));
		await vi.waitFor(() => expect(document.body.textContent).toContain('gone'));
		expect(net.calls.map((c) => c.method)).toEqual(['GET', 'DELETE', 'GET']);
	});

	// Two reloads overlap (a live update during a revoke's reload). Catches an older answer, success or failure, that
	// lands after the newer one and overwrites it.
	it('an older answer that arrives after a newer one is dropped', async () => {
		await open([connection()]);
		const pending: Deferred[] = [];
		net.pending = pending;
		void liveFns[0]();
		void liveFns[0]();
		expect(pending).toHaveLength(2);
		pending[1].resolve([connection({ id: 8 })]);
		await vi.waitFor(() => expect(rowOf(8)).not.toBeNull());
		pending[0].reject(new Error('late failure'));
		await settle();
		flushSync();
		expect(document.body.textContent).not.toContain('late failure');
		expect(rowOf(8)).not.toBeNull();

		void liveFns[0]();
		void liveFns[0]();
		pending[3].reject(new Error('newest failed'));
		await vi.waitFor(() => expect(document.body.textContent).toContain('Could not load connected apps: newest failed'));
		pending[2].resolve([connection({ id: 9 })]);
		await settle();
		flushSync();
		expect(document.body.textContent).toContain('Could not load connected apps: newest failed');
	});

	// An exchange writes no activity entry (the approval did, before the connection existed), and an end writes none.
	// Catches a list that waits for one: the visible-tab timer reloads it, and the minute clock ends a connection on time.
	it('reloads while the tab is visible and stops offering Revoke once a connection has ended', async () => {
		vi.useFakeTimers({ now: NOW, toFake: ['Date', 'setInterval', 'clearInterval'] });
		await open([connection({ ends_at: '2026-11-14T09:00:30Z' })]);
		expect(revokeButton(7)).not.toBeNull();
		expect(polls).toHaveLength(1);
		net.rows = [connection({ id: 8 }), connection({ ends_at: '2026-11-14T09:00:30Z' })];
		await polls[0].run();
		flushSync();
		expect(rowOf(8)).not.toBeNull();
		expect(revokeButton(7)).not.toBeNull();
		vi.advanceTimersByTime(60_000);
		flushSync();
		expect(revokeButton(7)).toBeNull();
		expect(rowOf(7).textContent).toContain('Ended 2026-11-14');
		expect(revokeButton(8)).not.toBeNull();
	});

	it('names a connection without a host by its name alone', async () => {
		await open([connection({ client_id: 'urn:reeve:developer-command', client_host: '', client_name: 'Developer command' })]);
		expect(rowOf(7).querySelector('.app')?.textContent?.trim()).toBe('Developer command');
	});

	it('says so when the list cannot be loaded', async () => {
		net.fail = new Error('boom');
		app = mount(ConnectedApps, { target: document.body });
		await vi.waitFor(() => expect(document.body.textContent).toContain('Could not load connected apps: boom'));
	});
});
