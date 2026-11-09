// @vitest-environment jsdom
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

const calls: { method: string; path: string; init: unknown }[] = [];
const user = (email: string, role: string, disabled = false) => ({ email, display_name: '', role, disabled });
const entry = (value: string, source: string) => ({
	value,
	source,
	also_in_config: false,
	allowed: true,
	added_by: source === 'config' ? null : 'dan@example.com',
	added_at: source === 'config' ? null : '2026-11-09T09:00:00Z'
});
const data = {
	users: [
		user('dan@example.com', 'admin'),
		user('ada@example.com', 'member'),
		user('pat@partner.org', 'admin'),
		user('old@gone.org', 'member')
	],
	access: {
		domains: [entry('example.com', 'config')],
		emails: [entry('pat@partner.org', 'database')],
		admins: [entry('dan@example.com', 'config'), entry('pat@partner.org', 'database')]
	}
};
vi.mock('#lib/api/client.ts', () => {
	const respond = (method: string) => async (path: string, init: unknown) => {
		calls.push({ method, path, init });
		return path === '/api/v1/users' ? data.users : data.access;
	};
	return {
		api: { GET: respond('GET'), POST: respond('POST'), DELETE: respond('DELETE') },
		unwrap: (p: Promise<unknown>) => p,
		ApiError: class extends Error {},
		Conflict: class extends Error {},
		Refused: class extends Error {}
	};
});
vi.mock('#lib/me.svelte.ts', async () => {
	const { SvelteMap } = await import('svelte/reactivity');
	const state = new SvelteMap<string, string>([['role', 'admin']]);
	return {
		setRole: (r: string) => state.set('role', r),
		isAdmin: () => state.get('role') === 'admin',
		session: { get me() { return { email: 'dan@example.com', role: state.get('role') }; } }
	};
});
const me = (await import('#lib/me.svelte.ts')) as unknown as { setRole: (r: string) => void };
const { default: Page } = await import('./+page.svelte');

let app: ReturnType<typeof mount> | null = null;
const baseUsers = [...data.users];
const baseAccess = structuredClone(data.access);
afterEach(() => {
	if (app) unmount(app);
	app = null;
	calls.length = 0;
	me.setRole('admin');
	data.users = [...baseUsers];
	data.access = structuredClone(baseAccess);
	document.body.innerHTML = '';
});
const write = () => calls.filter((c) => c.method !== 'GET').at(-1);
async function open() {
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => expect(document.querySelector('ul.rows li')).not.toBeNull());
	flushSync();
}
const rowOf = (email: string) => [...document.querySelectorAll('ul.rows li')].find((li) => li.querySelector('.email')?.textContent === email)!;
const buttons = (email: string) => [...rowOf(email).querySelectorAll('button')].map((b) => b.textContent);
const click = (email: string, label: string) =>
	([...rowOf(email).querySelectorAll('button')].find((b) => b.textContent === label) as HTMLButtonElement).click();

describe('Users page', () => {
	it('gives my own row no action', async () => {
		await open();
		expect(buttons('dan@example.com')).toEqual([]);
		expect(rowOf('dan@example.com').textContent).toContain('you');
	});

	it('offers Make admin and Disable on a member, and Make admin posts the grant', async () => {
		await open();
		expect(buttons('ada@example.com')).toEqual(['Make admin', 'Disable']);
		click('ada@example.com', 'Make admin');
		await vi.waitFor(() => expect(write()).toMatchObject({ method: 'POST', path: '/api/v1/access/admins', init: { body: { email: 'ada@example.com' } } }));
	});

	it('offers Revoke admin and Disable on a database admin, and Revoke deletes the grant', async () => {
		await open();
		expect(buttons('pat@partner.org')).toEqual(['Revoke admin', 'Disable']);
		click('pat@partner.org', 'Revoke admin');
		await vi.waitFor(() =>
			expect(write()).toMatchObject({ method: 'DELETE', path: '/api/v1/access/admins/{email}', init: { params: { path: { email: 'pat@partner.org' } } } })
		);
	});

	it('shows a configured admin other than me as locked, with no role action and no Disable', async () => {
		data.users.push(user('boss@example.com', 'admin'));
		data.access.admins.push(entry('boss@example.com', 'config'));
		await open();
		expect(rowOf('boss@example.com').textContent).toContain('admin (reeve.toml)');
		expect(buttons('boss@example.com')).toEqual([]);
	});

	it('keeps Disable on a user whose email is no longer allowed', async () => {
		await open();
		expect(buttons('old@gone.org')).toContain('Disable');
	});

	it('points to the Access page instead of saying roles are configuration-only', async () => {
		await open();
		expect(document.body.textContent).not.toContain('Roles come from the server configuration');
		expect(document.querySelector('a[href="/access"]')).not.toBeNull();
	});

	it('shows members the admins-only notice and asks the server nothing', async () => {
		me.setRole('member');
		app = mount(Page, { target: document.body });
		flushSync();
		expect(document.body.textContent).toContain('Admins only.');
		expect(calls).toHaveLength(0);
	});

	it('loads without a remount when a member is promoted while on the page', async () => {
		me.setRole('member');
		app = mount(Page, { target: document.body });
		flushSync();
		expect(calls).toHaveLength(0);
		me.setRole('admin');
		flushSync();
		await vi.waitFor(() => expect(document.querySelector('ul.rows li')).not.toBeNull());
		expect(buttons('ada@example.com')).toEqual(['Make admin', 'Disable']);
	});
});
