// @vitest-environment jsdom
// The war room strip: every mark on one line each, active first, with clear and renew in bulk behind an inline confirmation.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { WarRoomEntry } from '#lib/api/types.ts';
import WarRoom from './WarRoom.svelte';

const live = vi.hoisted(() => ({ fns: [] as (() => unknown)[] }));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => {
		live.fns.push(fn);
	}
}));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	return { page: fakePage({ url: '/' }).page };
});

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
	mark: { at: iso(-9 * day), by: 'dana@example.com', reason: 'Still burning', expires_at: iso(-2 * day), active: false, done: false }
};

const net = vi.hoisted(() => ({
	entries: [] as WarRoomEntry[],
	gets: 0,
	writes: [] as { method: string; path: string; body: unknown }[]
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(async ({ method, path, body }) => {
		if (method === 'GET' && path === '/api/v1/war-room') {
			net.gets++;
			return net.entries;
		}
		if (method !== 'GET') {
			net.writes.push({ method, path, body });
			if (path === '/api/v1/war-room/clear') {
				const sent = (body as { items: { key: string }[] }).items.map((r) => r.key);
				net.entries = net.entries.filter((e) => !sent.includes(e.item.key));
			}
			if (method === 'DELETE') net.entries = net.entries.filter((e) => !path.includes(`/${e.item.key}/`));
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
	net.gets = 0;
	net.writes.length = 0;
	live.fns.length = 0;
});

async function open(entries: WarRoomEntry[]) {
	net.entries = entries;
	app = mount(WarRoom, { target: document.body, props: { clock: () => now } });
	if (entries.length) await vi.waitFor(() => expect(document.body.querySelector('section[aria-label="War room"]')).not.toBeNull());
	else await new Promise((r) => setTimeout(r, 20));
	flushSync();
}
const button = (name: string) =>
	[...document.body.querySelectorAll('button')].find((b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === name || b.textContent?.trim() === name);
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
		expect(rows[0].querySelector('.why')!.textContent).toBe("'Customer escalation' · marked by dana · expires in 5 d");
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
		expect(net.gets).toBeGreaterThan(0);
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
		expect(document.body.querySelectorAll('li')[0].textContent).not.toContain('Re-mark');
		button('Re-mark ABC-311 Payments outage')!.click();
		flushSync();
		expect(document.body.querySelector('dialog h2')!.textContent).toBe('Mark ABC-311 urgent');
		const input = document.body.querySelector<HTMLInputElement>('dialog input')!;
		expect(input.value).toBe('Still burning');
		click('Mark urgent');
		await vi.waitFor(() => expect(net.writes).toHaveLength(1));
		expect(net.writes[0]).toEqual({ method: 'POST', path: '/api/v1/stories/ABC-311/urgent', body: { reason: 'Still burning' } });
	});

	it('links each row to its own page: the drawer for a story', async () => {
		await open([active, expired]);
		const hrefs = [...document.body.querySelectorAll('li a')].map((a) => a.getAttribute('href'));
		expect(hrefs).toEqual(['/initiatives/ABC-1', '/?story=ABC-311']);
	});

	it('names every row button by its item', async () => {
		await open([active, expired]);
		expect(button('Clear urgent mark on ABC-1 Payments rebuild')).toBeDefined();
		expect(button('Clear urgent mark on ABC-311 Payments outage')).toBeDefined();
		click('End war room');
		expect(button('Clear 2 marks')).toBeDefined();
	});

	it('moves focus into the confirmation and back to the opener on cancel', async () => {
		await open([active, expired]);
		click('End war room');
		await vi.waitFor(() => expect(document.activeElement?.hasAttribute('data-confirm')).toBe(true));
		click('Cancel');
		await vi.waitFor(() => expect(document.activeElement).toBe(button('End war room')));
	});

	it('sends what was confirmed, not a row that arrived after the confirmation opened', async () => {
		await open([active, expired]);
		click('End war room');
		const extra: WarRoomEntry = { ...active, item: { key: 'ABC-2', item: 'epic' }, title: 'Late one' };
		net.entries = [active, expired, extra];
		for (const f of live.fns) void f();
		await vi.waitFor(() => expect(document.body.querySelectorAll('li')).toHaveLength(3));
		expect(document.body.textContent).toContain('Clear 2 marks?');
		click('Clear');
		await vi.waitFor(() => expect(net.writes).toHaveLength(1));
		expect((net.writes[0].body as { items: unknown[] }).items).toEqual([active.item, expired.item]);
	});

	it('drops the confirmation when a confirmed row vanishes', async () => {
		await open([active, expired]);
		click('End war room');
		net.entries = [active];
		for (const f of live.fns) void f();
		await vi.waitFor(() => expect(button('End war room')).toBeDefined());
		expect(document.body.textContent).not.toContain('Clear 2 marks?');
	});

	it('drops a renew confirmation when a confirmed row is no longer expired', async () => {
		await open([active, expired]);
		click('Re-mark all expired');
		expect(document.body.textContent).toContain('Re-mark 1 expired?');
		net.entries = [active, { ...expired, mark: { ...expired.mark, active: true, expires_at: iso(14 * day) } }];
		for (const f of live.fns) void f();
		await vi.waitFor(() => expect(document.body.textContent).not.toContain('Re-mark 1 expired?'));
		expect(button('End war room')).toBeDefined();
	});

	it('moves focus to the next row after a row clear', async () => {
		await open([active, expired]);
		button('Clear urgent mark on ABC-1 Payments rebuild')!.click();
		await vi.waitFor(() => expect(document.body.querySelectorAll('li')).toHaveLength(1));
		await vi.waitFor(() => expect(document.activeElement?.closest('li')).toBe(document.body.querySelector('li')));
	});
});
