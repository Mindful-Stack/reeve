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
		fail: null as null | 'network' | 'forbidden' | { refuse: string },
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
		// A refusal in the server's own words, whatever they are: the page shows them as they come.
		if (fail) throw new ApiError(fail.refuse, 'validation');
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
/** The notices inside a form, and whether they stand above the form's buttons. */
const noticesIn = (form: HTMLFormElement) => [...form.querySelectorAll('.notice .text')].map((n) => n.textContent!.trim());
const noticeIsAboveButtons = (form: HTMLFormElement) => {
	const last = [...form.querySelectorAll('button')].at(-1)!;
	return !!(form.querySelector('.notice')!.compareDocumentPosition(last) & Node.DOCUMENT_POSITION_FOLLOWING);
};
const CONFLICT =
	'Someone else changed this template while you were editing. What you changed is still here; everything else now shows their version. Where you both changed the same thing, saving again replaces their change.';
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
		// The notice is in the form that was saved, above its buttons, and not at the top of the page.
		expect(noticesIn(editForm(2)!)).toEqual([CONFLICT]);
		expect(noticeIsAboveButtons(editForm(2)!)).toBe(true);
		expect(pageNotices()).toEqual([]);
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
		expect(noticesIn(editForm(2)!)).toEqual(['Failed to fetch']);
		expect(pageNotices()).toEqual([]);
		expect(textOf(editForm(2)!).value).toBe(mine);
		// The page reloaded and knows version 2. The editor must still save against the version it read.
		editForm(2)!.requestSubmit();
		await written(2);
		expect(writes()[1].init!.body).toStrictEqual({ version: 1, body: mine });
		// So the server refused it and their text is still stored. Only now is the admin told, and offered to replace it.
		expect(stored(2)).toMatchObject({ body: theirs, version: 2 });
		expect(noticesIn(editForm(2)!)).toEqual([CONFLICT]);
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

	// Catches the create's reset running after its reload: the reload moved the removed editor's text into the
	// new-template draft, and the create then emptied that draft and folded the form under a notice saying it was kept.
	it("keeps a removed editor's text that its create's reload moved into New template, and clears only the draft sent", async () => {
		await open('admin');
		const mine = 'One slice, rewritten.\n';
		const form = startEdit(3);
		type(nameOf(form), 'Thin slice');
		type(textOf(form), mine);
		const fresh = newForm()!;
		fresh.closest('details')!.open = true;
		type(nameOf(fresh), 'Spike');
		type(textOf(fresh), '## Question\n');
		// Another admin removes the edited template while the create is on its way: the reload after it finds it gone.
		db.meanwhile = () => (db.rows = db.rows.filter((r) => r.id !== 3));
		fresh.requestSubmit();
		await written(1);
		expect(stored(10)).toMatchObject({ level: 'initiative', name: 'Spike', body: '## Question\n' });
		expect(row(3)).toBeNull();
		// The sent draft is gone; the rescued text is in the form, open, at the removed template's level.
		expect(newForm()!.closest('details')!.open).toBe(true);
		expect(newForm()!.querySelector('select')!.value).toBe('epic');
		expect(nameOf(newForm()!).value).toBe('Thin slice');
		expect(textOf(newForm()!).value).toBe(mine);
		expect(pageNotices()).toEqual([REMOVED]);
		await vi.waitFor(() => expect(document.activeElement).toBe(nameOf(newForm()!)));
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

	it('says what removing a default template does to new items, at its level, and not for another template', async () => {
		const rows = seed();
		rows[2].is_default = true;
		await open('admin', rows);
		const confirmOf = (id: number) => {
			button(row(id)!, 'Remove')!.click();
			flushSync();
			return row(id)!.querySelector('.confirm')!.firstChild!.textContent!.trim();
		};
		expect(confirmOf(1)).toBe('Remove the template "Scenario"? Briefs already made from it stay as they are. New initiatives will then start with an empty brief.');
		expect(confirmOf(3)).toBe('Remove the template "Slice"? Briefs already made from it stay as they are. New epics will then start with an empty brief.');
		// The control: a template that is not the default.
		expect(confirmOf(2)).toBe('Remove the template "Tech debt"? Briefs already made from it stay as they are.');
	});

	it('is titled as its heading, and lets a name be as long as the server allows', async () => {
		await open('admin');
		expect(document.title).toBe('Brief templates · Reeve');
		expect(document.body.querySelector('h1')!.textContent).toBe('Brief templates');
		expect(nameOf(newForm()!).maxLength).toBe(100);
		expect(nameOf(startEdit(2)).maxLength).toBe(100);
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
		// A default's notice is the page's: no form is open for it.
		expect(pageNotices()).toEqual(['Only an admin can change templates.']);
		expect(marks()).toEqual([1]);
	});

	it("shows a failed removal's notice on the page", async () => {
		await open('admin');
		button(row(3)!, 'Remove')!.click();
		flushSync();
		db.fail = 'network';
		button(row(3)!, 'Remove template')!.click();
		await written(1);
		expect(pageNotices()).toEqual(['Failed to fetch']);
		expect(row(3)).not.toBeNull();
	});

	it('says in the edit form that a name or a text must not be blank, and sends nothing', async () => {
		await open('admin');
		const form = startEdit(2);
		type(nameOf(form), '  ');
		form.requestSubmit();
		flushSync();
		expect(noticesIn(editForm(2)!)).toEqual(['Name must not be blank.']);
		expect(noticeIsAboveButtons(editForm(2)!)).toBe(true);
		type(nameOf(form), 'Debt');
		type(textOf(form), ' \n');
		form.requestSubmit();
		flushSync();
		expect(noticesIn(editForm(2)!)).toEqual(['Template text must not be blank.']);
		expect(pageNotices()).toEqual([]);
		expect(writes()).toHaveLength(0);
		// A notice goes with its editor: cancelling and opening another shows none.
		button(editForm(2)!, 'Cancel')!.click();
		flushSync();
		expect(noticesIn(startEdit(1))).toEqual([]);
	});

	it("shows the server's refusal of a save in the edit form, in the server's words", async () => {
		await open('admin');
		const form = startEdit(2);
		type(nameOf(form), 'Scenario');
		db.fail = { refuse: 'whatever the server says about that name' };
		form.requestSubmit();
		await written(1);
		expect(noticesIn(editForm(2)!)).toEqual(['whatever the server says about that name']);
		expect(noticeIsAboveButtons(editForm(2)!)).toBe(true);
		expect(pageNotices()).toEqual([]);
		// The editor keeps what was typed, and a save that works takes the notice away with the form.
		expect(nameOf(editForm(2)!).value).toBe('Scenario');
		type(nameOf(editForm(2)!), 'Debt');
		editForm(2)!.requestSubmit();
		await written(2);
		expect(editForm(2)).toBeNull();
		expect(document.body.querySelector('.notice')).toBeNull();
	});

	it("shows the server's refusal of a new template in its form, in the server's words, and keeps the draft", async () => {
		await open('admin');
		const form = newForm()!;
		form.closest('details')!.open = true;
		type(nameOf(form), 'Scenario');
		type(textOf(form), '## Again\n');
		db.fail = { refuse: 'whatever the server says about that name' };
		form.requestSubmit();
		await written(1);
		expect(noticesIn(newForm()!)).toEqual(['whatever the server says about that name']);
		expect(noticeIsAboveButtons(newForm()!)).toBe(true);
		expect(pageNotices()).toEqual([]);
		expect(newForm()!.closest('details')!.open).toBe(true);
		expect(nameOf(newForm()!).value).toBe('Scenario');
		expect(textOf(newForm()!).value).toBe('## Again\n');
		// The next create that works leaves no notice behind.
		type(nameOf(newForm()!), 'Again');
		newForm()!.requestSubmit();
		await written(2);
		expect(row(10)).not.toBeNull();
		expect(document.body.querySelector('.notice')).toBeNull();
	});
});
describe('templates page: focus', () => {
	const active = () => document.activeElement;
	/** Presses a button as a person does: focus goes to it first. */
	function press(b: HTMLButtonElement) {
		b.focus();
		b.click();
		flushSync();
	}
	const focused = (el: () => Element | null | undefined) => vi.waitFor(() => expect(active()).toBe(el()));

	it('puts focus on the name when Edit opens the editor, and back on Edit when it is cancelled', async () => {
		await open('admin');
		press(button(row(2)!, 'Edit')!);
		await focused(() => nameOf(editForm(2)!));
		press(button(editForm(2)!, 'Cancel')!);
		await focused(() => button(row(2)!, 'Edit'));
	});

	it('puts focus back on Edit after a save, also one that changed nothing', async () => {
		await open('admin');
		press(button(row(2)!, 'Edit')!);
		await focused(() => nameOf(editForm(2)!));
		type(nameOf(editForm(2)!), 'Debt');
		button(editForm(2)!, 'Save')!.focus();
		editForm(2)!.requestSubmit();
		await written(1);
		await focused(() => button(row(2)!, 'Edit'));
		press(button(row(2)!, 'Edit')!);
		button(editForm(2)!, 'Save')!.focus();
		editForm(2)!.requestSubmit();
		flushSync();
		expect(writes()).toHaveLength(1);
		await focused(() => button(row(2)!, 'Edit'));
	});

	it('leaves focus where the admin moved it while a save was on its way', async () => {
		await open('admin');
		press(button(row(2)!, 'Edit')!);
		type(nameOf(editForm(2)!), 'Debt');
		editForm(2)!.requestSubmit();
		// The admin goes on to another row before the answer is there.
		const elsewhere = document.body.appendChild(document.createElement('button'));
		elsewhere.focus();
		expect(active()).toBe(elsewhere);
		await written(1);
		await new Promise((r) => setTimeout(r));
		expect(editForm(2)).toBeNull();
		expect(active()).toBe(elsewhere);
	});

	// Green before the page restored any focus: jsdom leaves focus on a control that is disabled, where a browser
	// drops it. It guards the restoring code against moving focus out of an editor that stays open.
	it('keeps focus in the editor when a save is refused', async () => {
		await open('admin');
		press(button(row(2)!, 'Edit')!);
		type(nameOf(editForm(2)!), 'Scenario');
		button(editForm(2)!, 'Save')!.focus();
		db.fail = { refuse: 'no' };
		editForm(2)!.requestSubmit();
		await written(1);
		await focused(() => button(editForm(2)!, 'Save'));
	});

	it("puts focus back on Remove when the removal's confirmation is cancelled", async () => {
		await open('admin');
		press(button(row(3)!, 'Remove')!);
		press(button(row(3)!, 'Cancel')!);
		await focused(() => button(row(3)!, 'Remove'));
	});

	// Catches focus dropped to the page by a failed removal: the confirmation went and nothing took focus back.
	it('puts focus back on Remove when a removal fails', async () => {
		await open('admin');
		press(button(row(3)!, 'Remove')!);
		db.fail = 'network';
		press(button(row(3)!, 'Remove template')!);
		await written(1);
		expect(row(3)).not.toBeNull();
		await focused(() => button(row(3)!, 'Remove'));
	});

	// Catches focus dropped to the page by a removal: its row went and nothing took focus. It goes to the next
	// row's first control, else the previous row's, else "New template".
	it('puts focus on a neighbouring row after a removal, or on New template when none is left', async () => {
		await open('admin');
		const first = (id: number) => row(id)!.querySelector('summary');
		press(button(row(1)!, 'Remove')!);
		press(button(row(1)!, 'Remove template')!);
		await written(1);
		await focused(() => first(2));
		// The last row: the previous one takes focus, across the levels.
		press(button(row(3)!, 'Remove')!);
		press(button(row(3)!, 'Remove template')!);
		await written(2);
		await focused(() => first(2));
		press(button(row(2)!, 'Remove')!);
		press(button(row(2)!, 'Remove template')!);
		await written(3);
		await focused(() => newForm()!.closest('details')!.querySelector('summary'));
	});

	// A guard, green before removals restored focus: the restoring code must not take focus the admin moved.
	it('leaves focus where the admin moved it while a removal was on its way', async () => {
		await open('admin');
		press(button(row(3)!, 'Remove')!);
		press(button(row(3)!, 'Remove template')!);
		const elsewhere = document.body.appendChild(document.createElement('button'));
		elsewhere.focus();
		await written(1);
		await new Promise((r) => setTimeout(r));
		expect(active()).toBe(elsewhere);
	});

	it('keeps focus on the default button, which is replaced by its opposite', async () => {
		await open('admin');
		press(button(row(2)!, 'Use as default')!);
		await written(1);
		await focused(() => button(row(2)!, 'Stop using as default'));
	});

	it('puts focus on the name of the new template that took the text of a removed one', async () => {
		await open('admin');
		press(button(row(3)!, 'Edit')!);
		await focused(() => nameOf(editForm(3)!));
		db.rows = db.rows.filter((r) => r.id !== 3);
		await liveFns[0]();
		flushSync();
		await focused(() => nameOf(newForm()!));
	});
});

