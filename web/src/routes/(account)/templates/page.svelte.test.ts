// @vitest-environment jsdom
// The templates page: every member reads the templates by level, each body rendered inside its row; an admin also
// creates, edits (only what changed, under the version read), marks and unmarks the default, and removes after a
// confirmation. The fake server applies every write, so the tests assert what the page shows afterwards.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

type Row = { id: number; level: 'initiative' | 'epic'; name: string; body: string; is_default: boolean; version: number };
type Init = { params?: { path?: { id: number } }; body?: unknown } | undefined;
type Call = { method: string; path: string; init: Init };

const { calls, db, liveFns } = vi.hoisted(() => ({
	calls: [] as Call[],
	/** The fake server: its rows, and what happens around the next request. */
	db: {
		rows: [] as Row[],
		nextId: 10,
		/** Runs once, just before the next write is judged: another admin's change that landed first. */
		meanwhile: null as null | (() => void),
		/** The next write fails this way instead of being applied. */
		fail: null as null | 'network' | 'forbidden',
		/** A list request takes its snapshot of the rows when it is made, then waits on the next gate, if any: an
		 *  answer that is already old when it arrives. */
		gates: [] as Promise<void>[]
	},
	liveFns: [] as (() => unknown)[]
}));

vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: (fn: () => unknown) => void liveFns.push(fn)
}));
vi.mock('#lib/api/client.ts', () => {
	class ApiError extends Error {
		code: string;
		constructor(message: string, code = 'internal') {
			super(message);
			this.code = code;
		}
	}
	class Conflict extends ApiError {
		current: number;
		constructor(current: number) {
			super('conflict', 'conflict');
			this.current = current;
		}
	}
	const copy = (r: Row): Row => ({ ...r });
	/** The list as the server orders it: initiative templates first, then by lowercased name, then by id. */
	const listed = (): Row[] =>
		[...db.rows]
			.sort((a, b) =>
				a.level !== b.level ? (a.level === 'initiative' ? -1 : 1) : a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.id - b.id
			)
			.map(copy);
	const respond = (method: string) => async (path: string, init: Init) => {
		calls.push({ method, path, init });
		if (method === 'GET') {
			const snapshot = listed();
			const gate = db.gates.shift();
			if (gate) await gate;
			return snapshot;
		}
		db.meanwhile?.();
		db.meanwhile = null;
		const fail = db.fail;
		db.fail = null;
		if (fail === 'network') throw new TypeError('Failed to fetch');
		if (fail === 'forbidden') throw new ApiError('forbidden', 'forbidden');
		if (method === 'POST' && path === '/api/v1/brief-templates') {
			const b = init!.body as { level: Row['level']; name: string; body: string };
			const row: Row = { id: db.nextId++, level: b.level, name: b.name.trim(), body: b.body, is_default: false, version: 1 };
			db.rows.push(row);
			return copy(row);
		}
		const id = init!.params!.path!.id;
		const row = db.rows.find((r) => r.id === id);
		if (!row) throw new ApiError('not found', 'not_found');
		if (method === 'PATCH' && path === '/api/v1/brief-templates/{id}') {
			const b = init!.body as { version: number; name?: string; body?: string };
			if (b.version !== row.version) throw new Conflict(row.version);
			if (b.name !== undefined) row.name = b.name.trim();
			if (b.body !== undefined) row.body = b.body;
			row.version += 1;
			return copy(row);
		}
		if (path === '/api/v1/brief-templates/{id}/default') {
			// Setting the mark takes it from the level's previous default, as the server does in one transaction.
			if (method === 'POST') for (const other of db.rows) if (other.level === row.level) other.is_default = false;
			row.is_default = method === 'POST';
			return copy(row);
		}
		if (method === 'DELETE' && path === '/api/v1/brief-templates/{id}') {
			db.rows = db.rows.filter((r) => r.id !== id);
			return {};
		}
		throw new Error(`unexpected ${method} ${path}`);
	};
	return {
		api: { GET: respond('GET'), POST: respond('POST'), PATCH: respond('PATCH'), DELETE: respond('DELETE') },
		unwrap: (p: Promise<unknown>) => p,
		ApiError,
		Conflict,
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
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	calls.length = 0;
	liveFns.length = 0;
	db.rows = [];
	db.nextId = 10;
	db.meanwhile = null;
	db.fail = null;
	db.gates.length = 0;
	me.setRole('admin');
});

const SCENARIO = '# Scenario\n\n| Step | Who |\n|---|---|\n| 1 | Dana |\n';
const seed = (): Row[] => [
	{ id: 1, level: 'initiative', name: 'Scenario', body: SCENARIO, is_default: true, version: 1 },
	{ id: 2, level: 'initiative', name: 'Tech debt', body: '## Debt\n', is_default: false, version: 1 },
	{ id: 3, level: 'epic', name: 'Slice', body: 'One slice.\n', is_default: false, version: 1 }
];
async function open(role: 'admin' | 'member', rows: Row[] = seed()) {
	me.setRole(role);
	db.rows = rows;
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('section[aria-label="Epic templates"]')).not.toBeNull();
	});
	expect(document.body.querySelectorAll('[data-template]')).toHaveLength(rows.length);
}
const row = (id: number) => document.body.querySelector(`[data-template="${id}"]`) as HTMLElement | null;
const stored = (id: number) => db.rows.find((r) => r.id === id);
const button = (root: ParentNode, name: string) => [...root.querySelectorAll('button')].find((b) => b.textContent?.trim() === name) as HTMLButtonElement | undefined;
const writes = () => calls.filter((c) => c.method !== 'GET');
/** The ids of the rows that show the Default mark in their summary, in page order. */
const marks = () =>
	[...document.body.querySelectorAll<HTMLElement>('[data-template]')].filter((a) => a.querySelector('summary .mark')).map((a) => Number(a.dataset.template));
/** The page's own controls. The rendered bodies are not searched: what is in them is the brief renderer's. */
const controls = () => document.body.querySelectorAll('article > .actions button, .confirm button, form');
/** The notices of the page itself: the ones that are in no form. */
const pageNotices = () => [...document.body.querySelectorAll('.notice .text')].filter((n) => !n.closest('form')).map((n) => n.textContent!.trim());
const newForm = () => document.body.querySelector('form[aria-label="New template"]') as HTMLFormElement | null;
const editForm = (id: number) => row(id)?.querySelector('form[aria-label="Edit template"]') as HTMLFormElement | null | undefined;
const nameOf = (form: HTMLFormElement) => form.querySelector('input') as HTMLInputElement;
const textOf = (form: HTMLFormElement) => form.querySelector('textarea') as HTMLTextAreaElement;
function type(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	el.dispatchEvent(new Event('change', { bubbles: true }));
	flushSync();
}
function startEdit(id: number): HTMLFormElement {
	button(row(id)!, 'Edit')!.click();
	flushSync();
	return editForm(id)!;
}
/** Waits for the write that was just started and for the reload after it. */
async function written(count: number) {
	await vi.waitFor(() => expect(writes()).toHaveLength(count));
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('[aria-busy="true"]')).toBeNull();
	});
}

describe('templates page', () => {
	it('shows a member every template by level, its body rendered inside its row, and no control', async () => {
		await open('member');
		const sections = [...document.body.querySelectorAll('section[aria-label]')].map((s) => s.getAttribute('aria-label'));
		expect(sections).toEqual(['Initiative templates', 'Epic templates']);
		// The name and the Default mark are in the summary; the body is inside the details.
		expect(row(1)!.querySelector('details > summary')!.textContent).toContain('Scenario');
		expect(marks()).toEqual([1]);
		// The body is rendered as a brief is: the Markdown table is a table, not pipes and dashes.
		expect(row(1)!.querySelector('details .body table')).not.toBeNull();
		expect(row(1)!.querySelector('details .body')!.textContent).not.toContain('|---|');
		expect(controls()).toHaveLength(0);
		expect(document.body.textContent).toContain('they can pick another one or none');
	});

	it('tells a member whose level has no template who can add one', async () => {
		await open('member', []);
		expect(document.body.textContent).toContain('No initiative templates yet. An admin can add one.');
		expect(document.body.textContent).toContain('No epic templates yet. An admin can add one.');
		expect(document.body.textContent).not.toContain('Add one with "New template" above.');
		expect(controls()).toHaveLength(0);
	});

	it('tells an admin with no template where to add one, and opens the form for it', async () => {
		await open('admin', []);
		expect(document.body.textContent).toContain('No initiative templates yet. Add one with "New template" above.');
		expect(document.body.textContent).toContain('No epic templates yet. Add one with "New template" above.');
		expect(document.body.textContent).not.toContain('An admin can add one.');
		expect(newForm()!.closest('details')!.open).toBe(true);
	});

	it('lets an admin add a template from the form above the lists, and shows the new row', async () => {
		await open('admin');
		const form = newForm()!;
		// The control of the test above: with templates there, the form starts closed.
		expect(form.closest('details')!.open).toBe(false);
		const firstList = document.body.querySelector('section[aria-label="Initiative templates"]')!;
		expect(form.compareDocumentPosition(firstList) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
		expect(form.textContent).toContain('Template text (Markdown)');
		expect(form.textContent).toContain("Avoid the words 'decided', 'Jira' and 'Confluence' in a template: Reeve would note on each initiative or epic made from it that its brief cites them without a required decision.");
		type(form.querySelector('select')!, 'epic');
		type(nameOf(form), 'Spike');
		type(textOf(form), '## Question\n');
		form.requestSubmit();
		await written(1);
		expect(writes()[0]).toMatchObject({ method: 'POST', path: '/api/v1/brief-templates', init: { body: { level: 'epic', name: 'Spike', body: '## Question\n' } } });
		// The row the server made is on the page, in its level's list, and the form is empty for the next one.
		const added = row(10)!;
		expect(added.closest('section')!.getAttribute('aria-label')).toBe('Epic templates');
		expect(added.querySelector('summary')!.textContent).toContain('Spike');
		expect(nameOf(newForm()!).value).toBe('');
		expect(textOf(newForm()!).value).toBe('');
	});

	it('moves the default mark and takes it off, and shows what the server holds afterwards', async () => {
		await open('admin');
		expect(marks()).toEqual([1]);
		expect(button(row(1)!, 'Use as default')).toBeUndefined();
		button(row(2)!, 'Use as default')!.click();
		await written(1);
		expect(writes()[0]).toMatchObject({ method: 'POST', path: '/api/v1/brief-templates/{id}/default', init: { params: { path: { id: 2 } } } });
		expect(marks()).toEqual([2]); // moved from the first template, not added beside it
		button(row(2)!, 'Stop using as default')!.click();
		await written(2);
		expect(writes()[1]).toMatchObject({ method: 'DELETE', path: '/api/v1/brief-templates/{id}/default', init: { params: { path: { id: 2 } } } });
		expect(marks()).toEqual([]);
		expect(button(row(2)!, 'Use as default')).toBeDefined();
	});

	it("saves only the field that changed, and takes a colleague's rename instead of writing the old name back", async () => {
		await open('admin');
		const mine = '## Debt\nWhat it costs.\n';
		type(textOf(startEdit(2)), mine);
		// A colleague renames the template before this save arrives.
		db.meanwhile = () => Object.assign(stored(2)!, { name: 'Debt', version: 2 });
		editForm(2)!.requestSubmit();
		await written(1);
		// Only the body was sent: the name was never touched here.
		expect(writes()[0]).toMatchObject({ method: 'PATCH', path: '/api/v1/brief-templates/{id}', init: { params: { path: { id: 2 } } } });
		expect(writes()[0].init!.body).toStrictEqual({ version: 1, body: mine });
		// The server refused it (the version was stale). The editor is still open with the text; the name it did
		// not change is the colleague's now; and the notice says what saving again does.
		expect(textOf(editForm(2)!).value).toBe(mine);
		expect(nameOf(editForm(2)!).value).toBe('Debt');
		expect(document.body.textContent).toContain('saving again replaces their change');
		editForm(2)!.requestSubmit();
		await written(2);
		expect(writes()[1].init!.body).toStrictEqual({ version: 2, body: mine });
		// Both changes are stored, and the page shows them.
		expect(stored(2)).toMatchObject({ name: 'Debt', body: mine, version: 3 });
		expect(editForm(2)).toBeNull();
		expect(row(2)!.querySelector('summary')!.textContent).toContain('Debt');
	});

	it("keeps the version it read after a failed save, so a colleague's edit is not replaced without the warning", async () => {
		await open('admin');
		const mine = '## Debt\nMine.\n';
		const theirs = '## Debt\nTheirs.\n';
		type(textOf(startEdit(2)), mine);
		// A colleague saves another text first; then this save is lost on the network.
		db.meanwhile = () => Object.assign(stored(2)!, { body: theirs, version: 2 });
		db.fail = 'network';
		editForm(2)!.requestSubmit();
		await written(1);
		expect(document.body.textContent).toContain('Failed to fetch');
		expect(document.body.textContent).not.toContain('saving again replaces their change');
		expect(textOf(editForm(2)!).value).toBe(mine);
		// The page reloaded and knows version 2. The editor must still save against the version it read.
		editForm(2)!.requestSubmit();
		await written(2);
		expect(writes()[1].init!.body).toStrictEqual({ version: 1, body: mine });
		// So the server refused it and their text is still stored. Only now is the admin told, and offered to replace it.
		expect(stored(2)).toMatchObject({ body: theirs, version: 2 });
		expect(document.body.textContent).toContain('saving again replaces their change');
		expect(textOf(editForm(2)!).value).toBe(mine);
		editForm(2)!.requestSubmit();
		await written(3);
		expect(writes()[2].init!.body).toStrictEqual({ version: 2, body: mine });
		expect(stored(2)).toMatchObject({ body: mine, version: 3 });
		expect(editForm(2)).toBeNull();
	});

	it("does not let a second Edit, or a new template, discard the editor that is open", async () => {
		await open('admin');
		// The control: with no editor open, every row can be edited.
		expect(button(row(1)!, 'Edit')!.disabled).toBe(false);
		type(textOf(startEdit(2)), 'half typed');
		// Every other row's Edit is off while this editor is open, and says why.
		for (const id of [1, 3]) {
			expect(button(row(id)!, 'Edit')!.disabled).toBe(true);
			expect(button(row(id)!, 'Edit')!.title).toBe('Save or cancel the template you are editing first.');
		}
		button(row(1)!, 'Edit')!.click();
		flushSync();
		expect(editForm(1)).toBeNull();
		expect(textOf(editForm(2)!).value).toBe('half typed');
		// A template made meanwhile leaves the editor and its text as they are.
		const form = newForm()!;
		form.closest('details')!.open = true;
		type(nameOf(form), 'Spike');
		type(textOf(form), '## Question\n');
		form.requestSubmit();
		await written(1);
		expect(row(10)).not.toBeNull();
		expect(textOf(editForm(2)!).value).toBe('half typed');
		expect(stored(2)!.body).toBe('## Debt\n');
		// Once the editor is closed the other rows can be edited again.
		button(editForm(2)!, 'Cancel')!.click();
		flushSync();
		expect(button(row(1)!, 'Edit')!.disabled).toBe(false);
		expect(button(row(1)!, 'Edit')!.title).toBe('');
	});

	const REMOVED = 'Someone else removed the template you were editing. Your text is now under "New template", so you can save it as a new template.';

	it("moves the admin's name and text into New template when a colleague removes the template being edited", async () => {
		await open('admin');
		const mine = 'One slice.\nAnd what it proves.\n';
		const form = startEdit(3);
		type(nameOf(form), 'Thin slice');
		type(textOf(form), mine);
		// The control: the form for a new template is folded, empty and set to the other level.
		expect(newForm()!.closest('details')!.open).toBe(false);
		expect(newForm()!.querySelector('select')!.value).toBe('initiative');
		db.rows = db.rows.filter((r) => r.id !== 3);
		await liveFns[0]();
		flushSync();
		expect(row(3)).toBeNull();
		expect(document.body.querySelector('form[aria-label="Edit template"]')).toBeNull();
		// What was typed is in the form for a new template, open, at the removed template's level.
		expect(newForm()!.closest('details')!.open).toBe(true);
		expect(newForm()!.querySelector('select')!.value).toBe('epic');
		expect(nameOf(newForm()!).value).toBe('Thin slice');
		expect(textOf(newForm()!).value).toBe(mine);
		expect(pageNotices()).toEqual([REMOVED]);
		// And it can be saved from there.
		newForm()!.requestSubmit();
		await written(1);
		expect(stored(10)).toMatchObject({ level: 'epic', name: 'Thin slice', body: mine });
		// The other rows can be edited again.
		expect(button(row(1)!, 'Edit')!.disabled).toBe(false);
	});

	it('keeps both texts when the template being edited is removed while a new template is half written', async () => {
		await open('admin');
		const fresh = newForm()!;
		fresh.closest('details')!.open = true;
		type(nameOf(fresh), 'Spike');
		type(textOf(fresh), '## Question\n');
		type(textOf(startEdit(3)), 'One slice, rewritten.\n');
		// The save itself finds the template gone.
		db.meanwhile = () => (db.rows = db.rows.filter((r) => r.id !== 3));
		editForm(3)!.requestSubmit();
		await written(1);
		expect(row(3)).toBeNull();
		// The half-written template keeps its level and name; the editor's text follows its own.
		expect(newForm()!.querySelector('select')!.value).toBe('initiative');
		expect(nameOf(newForm()!).value).toBe('Spike');
		expect(textOf(newForm()!).value).toBe('## Question\n\nOne slice, rewritten.\n');
		expect(pageNotices()).toEqual([REMOVED]);
	});

	it('shows a rendered preview of the text while it is edited', async () => {
		await open('admin');
		const form = startEdit(2);
		expect(form.textContent).toContain('Template text (Markdown)');
		// The control: this template's text has no table.
		expect(form.querySelector('.preview')).not.toBeNull();
		expect(form.querySelector('.preview table')).toBeNull();
		type(textOf(form), '| Step | Who |\n|---|---|\n| 1 | Dana |\n');
		expect(form.querySelector('.preview table')).not.toBeNull();
		// A blank text has no preview.
		type(textOf(form), ' \n');
		expect(form.querySelector('.preview')).toBeNull();
	});

	it('removes only after a confirmation, and the row is gone', async () => {
		await open('admin');
		button(row(3)!, 'Remove')!.click();
		flushSync();
		expect(writes()).toHaveLength(0);
		expect(row(3)!.textContent).toContain('Remove the template "Slice"?');
		button(row(3)!, 'Cancel')!.click();
		flushSync();
		expect(writes()).toHaveLength(0);
		expect(row(3)!.querySelector('.confirm')).toBeNull();
		button(row(3)!, 'Remove')!.click();
		flushSync();
		button(row(3)!, 'Remove template')!.click();
		await written(1);
		expect(writes()[0]).toMatchObject({ method: 'DELETE', path: '/api/v1/brief-templates/{id}', init: { params: { path: { id: 3 } } } });
		expect(row(3)).toBeNull();
		expect(document.body.querySelectorAll('[data-template]')).toHaveLength(2);
	});

	it('takes every control from a viewer who stops being an admin, whatever is open', async () => {
		await open('admin');
		// The form for a new template.
		expect(newForm()).not.toBeNull();
		me.setRole('member');
		flushSync();
		expect(newForm()).toBeNull();
		me.setRole('admin');
		flushSync();
		// An open editor: gone, the member's view of the row in its place, and it does not come back by itself.
		type(textOf(startEdit(2)), 'half typed');
		me.setRole('member');
		flushSync();
		expect(editForm(2)).toBeNull();
		expect(row(2)!.querySelector('details .body')).not.toBeNull();
		me.setRole('admin');
		flushSync();
		expect(editForm(2)).toBeNull();
		// An open confirmation.
		button(row(3)!, 'Remove')!.click();
		flushSync();
		expect(row(3)!.querySelector('.confirm')).not.toBeNull();
		me.setRole('member');
		flushSync();
		expect(controls()).toHaveLength(0);
		expect(writes()).toHaveLength(0);
	});

	it('shows the newest list when two reloads overlap', async () => {
		await open('admin');
		let release!: () => void;
		db.gates.push(new Promise<void>((r) => (release = r)));
		// A live reload asks for the list, with all three rows in its answer, and is slow.
		const slow = liveFns[0]();
		// Meanwhile a colleague removes "Slice", and a second live reload answers at once without it.
		db.rows = db.rows.filter((r) => r.id !== 3);
		await liveFns[0]();
		flushSync();
		expect(row(3)).toBeNull();
		// The older answer arrives last and must not put the row back.
		release();
		await slow;
		flushSync();
		expect(row(3)).toBeNull();
		expect(document.body.querySelectorAll('[data-template]')).toHaveLength(2);
	});

	it('says who may change templates when the server refuses a write', async () => {
		await open('admin');
		db.fail = 'forbidden';
		button(row(2)!, 'Use as default')!.click();
		await written(1);
		expect(document.body.textContent).toContain('Only an admin can change templates.');
		expect(marks()).toEqual([1]);
	});
});
