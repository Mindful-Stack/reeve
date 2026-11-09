// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({ role: 'admin', email: 'dan@example.com' }));
const calls: { method: string; path: string; init: unknown }[] = [];
const access = {
	domains: [{ value: 'example.com', source: 'config', also_in_config: false, allowed: true, added_by: null, added_at: null }],
	emails: [{ value: 'guest@other.org', source: 'database', also_in_config: false, allowed: true, added_by: 'dan@example.com', added_at: '2026-11-09T09:00:00Z' }],
	admins: [
		{ value: 'dan@example.com', source: 'config', also_in_config: false, allowed: true, added_by: null, added_at: null },
		{ value: 'eve@gone.org', source: 'database', also_in_config: false, allowed: false, added_by: 'dan@example.com', added_at: '2026-11-09T09:00:00Z' }
	]
};
vi.mock('#lib/api/client.ts', () => {
	const respond = (method: string) => async (path: string, init: unknown) => {
		calls.push({ method, path, init });
		return path === '/api/v1/users' ? [] : access;
	};
	return {
		api: { GET: respond('GET'), POST: respond('POST'), DELETE: respond('DELETE') },
		unwrap: (p: Promise<unknown>) => p,
		ApiError: class extends Error {},
		Conflict: class extends Error {},
		Refused: class extends Error {}
	};
});
vi.mock('#lib/me.svelte.ts', () => ({
	isAdmin: () => state.role === 'admin',
	session: { get me() { return { email: state.email, role: state.role }; } }
}));
const { default: Page } = await import('./+page.svelte');

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	calls.length = 0;
	state.role = 'admin';
	document.body.innerHTML = '';
});
const write = () => calls.filter((c) => c.method !== 'GET').at(-1);
async function open() {
	app = mount(Page, { target: document.body });
	// wait for the loads to settle rather than counting microtask ticks
	if (state.role === 'admin') await vi.waitFor(() => expect(document.querySelector('section li')).not.toBeNull());
	flushSync();
}
/** The row in one section: `example.com` also appears inside the guest row's "added by dan@example.com". */
const rowOf = (section: string, text: string) =>
	[...document.querySelectorAll(`section[aria-label="${section}"] li`)].find((li) => li.textContent?.includes(text))!;

describe('Access page', () => {
	it('shows configuration rows locked and database rows with their action', async () => {
		await open();
		expect(rowOf('Domains', 'example.com').textContent).toContain('from reeve.toml');
		expect(rowOf('Domains', 'example.com').querySelector('button')).toBeNull();
		expect(rowOf('Guests', 'guest@other.org').querySelector('button')?.textContent).toBe('Remove');
	});

	it('flags an inert grant and keeps its Revoke; my own admin row has no action', async () => {
		await open();
		expect(rowOf('Admins', 'eve@gone.org').textContent).toContain('not allowed: grants nothing until their email is allowed again');
		expect(rowOf('Admins', 'eve@gone.org').querySelector('button')?.textContent).toBe('Revoke');
		expect(rowOf('Admins', 'dan@example.com').querySelector('button')).toBeNull();
	});

	it('posts a guest, and with "also admin" posts a grant instead', async () => {
		await open();
		const form = document.querySelector('form[aria-label="Allow a guest"]') as HTMLFormElement;
		(form.querySelector('input[type=email]') as HTMLInputElement).value = 'new@other.org';
		form.querySelector('input[type=email]')!.dispatchEvent(new Event('input'));
		form.requestSubmit();
		await vi.waitFor(() => expect(write()).toMatchObject({ method: 'POST', path: '/api/v1/access/entries', init: { body: { kind: 'email', value: 'new@other.org' } } }));
		(form.querySelector('input[type=email]') as HTMLInputElement).value = 'boss@other.org';
		form.querySelector('input[type=email]')!.dispatchEvent(new Event('input'));
		(form.querySelector('input[type=checkbox]') as HTMLInputElement).click();
		form.requestSubmit();
		await vi.waitFor(() => expect(write()).toMatchObject({ method: 'POST', path: '/api/v1/access/admins', init: { body: { email: 'boss@other.org' } } }));
	});

	it('asks inline before removing, then deletes', async () => {
		await open();
		(rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement).click();
		flushSync();
		expect(document.body.textContent).toContain('Remove guest@other.org? They lose access on their next request.');
		const yes = [...document.querySelectorAll('button')].find((b) => b.textContent === 'Yes, remove') as HTMLButtonElement;
		yes.click();
		await vi.waitFor(() => expect(write()).toMatchObject({ method: 'DELETE', path: '/api/v1/access/entries/{kind}/{value}', init: { params: { path: { kind: 'email', value: 'guest@other.org' } } } }));
	});

	it('shows members the admins-only notice and asks the server nothing', async () => {
		state.role = 'member';
		await open();
		expect(document.body.textContent).toContain('Admins only.');
		expect(calls).toHaveLength(0);
	});
});
