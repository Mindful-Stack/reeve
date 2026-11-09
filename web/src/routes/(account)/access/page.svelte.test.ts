// @vitest-environment jsdom
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

const state = vi.hoisted(() => ({
	role: 'admin',
	email: 'dan@example.com',
	/** Replaces the data a read returns. */
	read: null as unknown,
	/** Makes the next write throw this. */
	fail: null as Error | null,
	/** Makes the users read throw. */
	failUsers: null as Error | null
}));
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
		if (path === '/api/v1/users') {
			if (state.failUsers) throw state.failUsers;
			return [];
		}
		if (method === 'GET') return state.read ?? access;
		if (state.fail) throw state.fail;
		// A write answers with a different list than a read, so a page that ignores it shows.
		return { domains: [], emails: [], admins: [{ ...access.admins[0] }] };
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
	state.email = 'dan@example.com';
	state.read = null;
	state.fail = null;
	state.failUsers = null;
	document.body.innerHTML = '';
});
const entry = (value: string, source: 'config' | 'database', extra = {}) => ({
	value,
	source,
	also_in_config: false,
	allowed: true,
	added_by: source === 'database' ? 'dan@example.com' : null,
	added_at: source === 'database' ? '2026-11-09T09:00:00Z' : null,
	...extra
});
const button = (label: string) => [...document.querySelectorAll('button')].find((b) => b.textContent === label) as HTMLButtonElement;
const fill = (form: HTMLFormElement, selector: string, value: string) => {
	const input = form.querySelector(selector) as HTMLInputElement;
	input.value = value;
	input.dispatchEvent(new Event('input'));
	return input;
};
const formOf = (label: string) => document.querySelector(`form[aria-label="${label}"]`) as HTMLFormElement;
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

	it('marks a database admin row that is also in reeve.toml and keeps its Revoke', async () => {
		const base = { allowed: true, added_by: 'dan@example.com', added_at: '2026-11-09T09:00:00Z' };
		const extra = [
			{ value: 'boss@example.com', source: 'config', also_in_config: false, allowed: true, added_by: null, added_at: null },
			{ value: 'boss@example.com', source: 'database', also_in_config: true, ...base }
		];
		access.admins.push(...extra);
		try {
			await open();
			const dbRow = [...document.querySelectorAll('section[aria-label="Admins"] li')].find(
				(li) => li.textContent?.includes('boss@example.com') && li.textContent.includes('added by')
			)!;
			expect(dbRow.textContent).toContain('also in reeve.toml');
			expect(dbRow.querySelector('button')?.textContent).toBe('Revoke');
		} finally {
			access.admins.splice(-extra.length, extra.length);
		}
	});

	it('sends nothing before Yes, and Cancel closes the dialog and returns focus to Remove', async () => {
		await open();
		const remove = rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement;
		remove.click();
		await tick();
		expect(document.querySelector('[role=alertdialog]')?.getAttribute('aria-label')).toBe('Confirm removal');
		expect(document.activeElement?.textContent).toBe('Yes, remove');
		expect(write()).toBeUndefined();
		button('Cancel').click();
		await tick();
		await tick();
		expect(document.querySelector('[role=alertdialog]')).toBeNull();
		expect(write()).toBeUndefined();
		expect(document.activeElement).toBe(remove);
	});

	it('Escape cancels the dialog', async () => {
		await open();
		(rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement).click();
		await tick();
		document.querySelector('[role=alertdialog]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		await tick();
		expect(document.querySelector('[role=alertdialog]')).toBeNull();
		expect(write()).toBeUndefined();
	});

	it('shows what a write answered, not what the earlier read said', async () => {
		await open();
		(rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement).click();
		await tick();
		button('Yes, remove').click();
		await vi.waitFor(() => expect(document.body.textContent).not.toContain('guest@other.org'));
	});

	it('revokes through the dialog with a DELETE to the admin path', async () => {
		await open();
		(rowOf('Admins', 'eve@gone.org').querySelector('button') as HTMLButtonElement).click();
		await tick();
		expect(document.querySelector('[role=alertdialog]')?.getAttribute('aria-label')).toBe('Confirm revoke');
		expect(write()).toBeUndefined();
		button('Yes, revoke').click();
		await vi.waitFor(() =>
			expect(write()).toMatchObject({ method: 'DELETE', path: '/api/v1/access/admins/{email}', init: { params: { path: { email: 'eve@gone.org' } } } })
		);
	});

	it('says an admin also in reeve.toml stays admin when the database grant is revoked', async () => {
		state.read = { domains: [entry('example.com', 'config')], emails: [], admins: [entry('dan@example.com', 'config'), entry('boss@example.com', 'database', { also_in_config: true })] };
		await open();
		(rowOf('Admins', 'boss@example.com').querySelector('button') as HTMLButtonElement).click();
		await tick();
		expect(document.body.textContent).toContain('Revoke admin from boss@example.com? They stay admin through reeve.toml.');
	});

	it('words a domain removal as a domain', async () => {
		state.read = { domains: [entry('example.com', 'config'), entry('partner.org', 'database')], emails: [], admins: [entry('dan@example.com', 'config')] };
		await open();
		(rowOf('Domains', 'partner.org').querySelector('button') as HTMLButtonElement).click();
		await tick();
		expect(document.body.textContent).toContain('Remove @partner.org? 0 listed users lose access on their next request.');
	});

	it('has no action on my own database admin row, but does on another', async () => {
		state.email = 'pat@partner.org';
		state.read = {
			domains: [entry('partner.org', 'database')],
			emails: [],
			admins: [entry('pat@partner.org', 'database'), entry('ana@partner.org', 'database')]
		};
		await open();
		expect(rowOf('Admins', 'pat@partner.org').querySelector('button')).toBeNull();
		expect(rowOf('Admins', 'ana@partner.org').querySelector('button')?.textContent).toBe('Revoke');
	});

	it('hides Remove on the only entry that allows me', async () => {
		state.email = 'pat@partner.org';
		state.read = { domains: [], emails: [entry('pat@partner.org', 'database'), entry('guest@other.org', 'database')], admins: [entry('pat@partner.org', 'config')] };
		await open();
		expect(rowOf('Guests', 'pat@partner.org').querySelector('button')).toBeNull();
		expect(rowOf('Guests', 'guest@other.org').querySelector('button')?.textContent).toBe('Remove');
	});

	it('keeps the input filled and shows the error when a write fails', async () => {
		state.fail = new Error('invalid request: not a valid email address');
		await open();
		const form = document.querySelector('form[aria-label="Allow a guest"]') as HTMLFormElement;
		const input = form.querySelector('input[type=email]') as HTMLInputElement;
		input.value = 'new@other.org';
		input.dispatchEvent(new Event('input'));
		form.requestSubmit();
		await vi.waitFor(() => expect(document.querySelector('[role=alert]')?.textContent).toContain('invalid request: not a valid email address'));
		expect(input.value).toBe('new@other.org');
	});

	it('clears the guest input and the also-admin box after a successful Allow, and refocuses the input', async () => {
		await open();
		const form = formOf('Allow a guest');
		const input = fill(form, 'input[type=email]', 'boss@other.org');
		const box = form.querySelector('input[type=checkbox]') as HTMLInputElement;
		box.click();
		expect(box.checked).toBe(true);
		form.requestSubmit();
		await vi.waitFor(() => expect(input.value).toBe(''));
		expect(box.checked).toBe(false);
		await vi.waitFor(() => expect(document.activeElement).toBe(input));
	});

	it('clears the domain input after a successful Allow and refocuses it', async () => {
		await open();
		const form = formOf('Allow a domain');
		const input = fill(form, 'input', 'partner.org');
		form.requestSubmit();
		await vi.waitFor(() => expect(input.value).toBe(''));
		await vi.waitFor(() => expect(document.activeElement).toBe(input));
	});

	it('clears the admin input after a successful Grant and refocuses it', async () => {
		await open();
		const form = formOf('Grant admin');
		const input = fill(form, 'input[type=email]', 'boss@other.org');
		form.requestSubmit();
		await vi.waitFor(() => expect(input.value).toBe(''));
		await vi.waitFor(() => expect(document.activeElement).toBe(input));
	});

	it('clears the input and says the list could not be reloaded when the write worked but the users read failed', async () => {
		await open();
		state.failUsers = new Error('network down');
		const form = formOf('Allow a domain');
		const input = fill(form, 'input', 'partner.org');
		form.requestSubmit();
		await vi.waitFor(() => expect(document.querySelector('[role=alert]')?.textContent).toContain('could not be reloaded'));
		expect(input.value).toBe('');
	});

	it('moves focus to the section first input when Yes removes the row', async () => {
		await open();
		(rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement).click();
		await tick();
		button('Yes, remove').click();
		await vi.waitFor(() => expect(document.body.textContent).not.toContain('guest@other.org'));
		await vi.waitFor(() => expect(document.activeElement).toBe(document.querySelector('section[aria-label="Guests"] input')));
	});

	it('returns focus to the opener after Escape', async () => {
		await open();
		const remove = rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement;
		remove.click();
		await tick();
		document.querySelector('[role=alertdialog]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
		await vi.waitFor(() => expect(document.activeElement).toBe(remove));
	});

	it('does not steal focus the user moved elsewhere before Cancel', async () => {
		await open();
		const remove = rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement;
		remove.click();
		await tick();
		const elsewhere = document.querySelector('section[aria-label="Domains"] input') as HTMLInputElement;
		elsewhere.focus();
		button('Cancel').click();
		await tick();
		await tick();
		expect(document.querySelector('[role=alertdialog]')).toBeNull();
		expect(document.activeElement).toBe(elsewhere);
	});

	it('points aria-describedby at the question text', async () => {
		await open();
		(rowOf('Guests', 'guest@other.org').querySelector('button') as HTMLButtonElement).click();
		await tick();
		const dlg = document.querySelector('[role=alertdialog]')!;
		const target = document.getElementById(dlg.getAttribute('aria-describedby')!);
		expect(target?.textContent).toContain('Remove guest@other.org?');
	});

	it('has no Revoke on my own row when the session email is mixed case', async () => {
		state.email = 'Pat@Partner.org';
		state.read = { domains: [entry('partner.org', 'database')], emails: [], admins: [entry('pat@partner.org', 'database'), entry('ana@partner.org', 'database')] };
		await open();
		expect(rowOf('Admins', 'pat@partner.org').querySelector('button')).toBeNull();
		expect(rowOf('Admins', 'ana@partner.org').querySelector('button')?.textContent).toBe('Revoke');
	});

	it('shows members the admins-only notice and asks the server nothing', async () => {
		state.role = 'member';
		await open();
		expect(document.body.textContent).toContain('Admins only.');
		expect(calls).toHaveLength(0);
	});
});
