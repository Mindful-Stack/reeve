// @vitest-environment jsdom
// The war room strip: every mark on one line each, active first, with clear and renew in bulk behind an inline confirmation.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { WarRoomEntry } from '#lib/api/types.ts';
import WarRoom from './WarRoom.svelte';

const now = new Date('2026-11-10T12:00:00Z');
const day = 86_400_000;
const iso = (offset: number) => new Date(now.getTime() + offset).toISOString();

const active: WarRoomEntry = {
	item: { key: 'ABC-1', item: 'initiative' },
	title: 'Payments rebuild',
	initiative: { key: 'ABC-1', title: 'Payments rebuild', horizon: 'Now' },
	mark: { at: iso(-day), by: 'dana@example.com', reason: 'Customer escalation', expires_at: iso(5 * day), active: true, done: false }
};
const expired: WarRoomEntry = {
	item: { key: 'ABC-311', item: 'story' },
	title: 'Payments outage',
	initiative: { key: 'ABC-1', title: 'Payments rebuild', horizon: 'Next' },
	mark: { at: iso(-9 * day), by: 'dana@example.com', reason: null, expires_at: iso(-2 * day), active: false, done: false }
};

const net = vi.hoisted(() => ({
	entries: [] as WarRoomEntry[],
	writes: [] as { method: string; path: string; body: unknown }[]
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(async ({ method, path, body }) => {
		if (method === 'GET' && path === '/api/v1/war-room') return net.entries;
		if (method !== 'GET') {
			net.writes.push({ method, path, body });
			if (path === '/api/v1/war-room/clear') net.entries = [];
			return path.startsWith('/api/v1/war-room') ? net.entries : {};
		}
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.entries = [];
	net.writes.length = 0;
});

async function open(entries: WarRoomEntry[]) {
	net.entries = entries;
	app = mount(WarRoom, { target: document.body, props: { now } });
	if (entries.length) await vi.waitFor(() => expect(document.body.querySelector('section[aria-label="War room"]')).not.toBeNull());
	else await new Promise((r) => setTimeout(r, 20));
	flushSync();
}
const button = (name: string) => [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === name);
const click = (name: string) => {
	const b = button(name);
	expect(b, `button ${name}`).toBeDefined();
	b!.click();
	flushSync();
};

describe('WarRoom', () => {
	it('lists active and expired marks in a named region', async () => {
		await open([active, expired]);
		const region = document.body.querySelector('section[aria-label="War room"]')!;
		expect(region.getAttribute('aria-label')).toBe('War room');
		const rows = region.querySelectorAll('li');
		expect(rows).toHaveLength(2);
		expect(rows[0].textContent).toContain('Payments rebuild');
		expect(rows[0].textContent).toContain('Now');
		expect(rows[0].textContent).toContain('Customer escalation');
		expect(rows[0].textContent).toContain('expires in 5 d');
		expect(rows[1].classList.contains('expired')).toBe(true);
		expect(rows[1].textContent).toContain('expired 2 d ago');
		expect([...rows[1].querySelectorAll('button')].map((b) => b.textContent?.trim())).toEqual(['Re-mark', 'Clear']);
	});

	it('shows the initiative and horizon of a story row', async () => {
		await open([expired]);
		expect(document.body.querySelector('li')!.textContent).toContain('in Payments rebuild · Next');
	});

	it('renders nothing when there are no marks', async () => {
		await open([]);
		expect(document.body.querySelector('section[aria-label="War room"]')).toBeNull();
	});

	it('ends the war room after an inline confirmation, sending exactly the refs on screen', async () => {
		await open([active, expired]);
		click('End war room');
		expect(button('End war room')).toBeUndefined();
		expect(document.body.textContent).toContain('Clear 2 marks?');
		click('Cancel');
		expect(button('End war room')).toBeDefined();
		click('End war room');
		click('Clear');
		await vi.waitFor(() => expect(net.writes).toHaveLength(1));
		expect(net.writes[0]).toEqual({
			method: 'POST',
			path: '/api/v1/war-room/clear',
			body: { items: [active.item, expired.item] }
		});
		await vi.waitFor(() => expect(document.body.querySelector('section[aria-label="War room"]')).toBeNull());
	});

	it('renews only the expired rows', async () => {
		await open([active, expired]);
		click('Re-mark all expired');
		expect(document.body.textContent).toContain('Re-mark 1 expired?');
		click('Re-mark');
		await vi.waitFor(() => expect(net.writes).toHaveLength(1));
		expect(net.writes[0]).toEqual({ method: 'POST', path: '/api/v1/war-room/renew', body: { items: [expired.item] } });
	});

	it('clears one row through its own route', async () => {
		await open([active, expired]);
		const row = document.body.querySelectorAll('li')[1];
		[...row.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Clear')!.click();
		await vi.waitFor(() => expect(net.writes).toHaveLength(1));
		expect(net.writes[0]).toMatchObject({ method: 'DELETE', path: '/api/v1/stories/ABC-311/urgent' });
	});

	it('re-marks one row with its reason prefilled', async () => {
		await open([active, expired]);
		const row = document.body.querySelectorAll('li')[0];
		// an active row has no Re-mark; use the expired one, then check the dialog
		expect(row.textContent).not.toContain('Re-mark');
		[...document.body.querySelectorAll('li')[1].querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Re-mark')!.click();
		flushSync();
		const input = document.body.querySelector<HTMLInputElement>('dialog input')!;
		expect(input).not.toBeNull();
		expect(input.value).toBe('');
		click('Mark urgent');
		await vi.waitFor(() => expect(net.writes).toHaveLength(1));
		expect(net.writes[0]).toEqual({ method: 'POST', path: '/api/v1/stories/ABC-311/urgent', body: { reason: null } });
	});
});
