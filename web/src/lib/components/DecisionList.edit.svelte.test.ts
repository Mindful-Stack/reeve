// @vitest-environment jsdom
// The decision editor over a stubbed network: what an unchanged or blank save does not send, and how a 409 merges the
// draft with the reloaded decision. The props are `$state`, so `onchanged` replaces the list as a page's reload does.
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BlockedStory, QueueGroup, RequiredDecision } from '#lib/api/types.ts';
import { decision } from '#lib/test/fixtures.ts';
import DecisionList from './DecisionList.svelte';

type Sent = { method: string; path: string; body: unknown };
// `reply` answers each request with a status and an envelope.
const net = vi.hoisted(() => {
	const sent: Sent[] = [];
	const ok = (value: unknown) => ({ status: 200, body: { outcome: 'ok', value } });
	const unexpected = (r: Sent): { status: number; body: unknown } => {
		throw new Error(`unexpected request ${r.method} ${r.path}`);
	};
	const replies = { reply: unexpected, unexpected, ok, gate: null as null | Promise<void> };
	return { sent, replies };
});
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, json } = await import('#lib/test/net.ts');
	installFetch(async ({ method, path, body }) => {
		const r = { method, path, body };
		net.sent.push(r);
		if (net.replies.gate) await net.replies.gate;
		const answer = net.replies.reply(r);
		return json(answer.body, answer.status);
	});
	return orig();
});

const open = decision({
	id: 1,
	on: 'RV-1',
	title: 'Pick a queue',
	decider: 'dan@example.com',
	due: '2026-11-30'
});
const closed = decision({
	...open,
	id: 2,
	title: 'Pick a cache',
	closed: { at: '2026-10-02T00:00:00Z', by: 'dan@example.com', reason: 'done' }
});

/** What the next `onchanged` loads; a test sets it to stage a colleague's change. */
let reloaded: RequiredDecision[] = [];
const props = $state({
	decisions: [] as RequiredDecision[],
	onchanged: async () => {
		props.decisions = reloaded;
	}
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	vi.useRealTimers();
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.sent.length = 0;
	net.replies.reply = net.replies.unexpected;
	net.replies.gate = null;
});

const rows = () => [...document.body.querySelectorAll('.decisions > li')];
const button = (root: Element, name: string) =>
	[...root.querySelectorAll('button')].find((b) => b.textContent?.trim() === name) as HTMLButtonElement | undefined;
const field = (form: Element, name: string) =>
	[...form.querySelectorAll('label')]
		.find((l) => l.textContent?.trim().startsWith(name))!
		.querySelector('input') as HTMLInputElement;
const editor = () => document.body.querySelector('form.edit');
const alert = () => document.body.querySelector('[role="alert"]');
/** The editor's own error: inside `form.edit`, next to the fields, not at the top of the row above the thread. */
const editAlert = () => editor()?.querySelector('[role="alert"]') ?? null;
const patches = () => net.sent.filter((r) => r.method === 'PATCH').map((r) => [r.path, r.body]);
/** The row's Actions menu trigger: Edit, Decide and Withdraw are reached through it. */
const actions = (row: Element) => row.querySelector<HTMLButtonElement>(`button[aria-label^="Actions for "]`);
const menuitem = (name: string) =>
	[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.trim() === name);
/** Opens the row's editor the way a user does: its Actions menu, then Edit. */
function edit(row: Element) {
	const title = row.querySelector('.title')?.textContent?.trim();
	const trigger = row.querySelector<HTMLButtonElement>(`button[aria-label="Actions for ${title} on RV-1"]`)!;
	trigger.click();
	flushSync();
	menuitem('Edit')!.click();
	flushSync();
}
/** Expands the row's discussion (its thread and comment box). */
const discussion = (row: Element) => row.querySelector<HTMLButtonElement>('button[data-field="discussion"]')!;
function show(list: RequiredDecision[]) {
	props.decisions = list;
	reloaded = list;
	app = mount(DecisionList, { target: document.body, props });
	flushSync();
}
function enter(input: HTMLInputElement, value: string) {
	input.value = value;
	input.dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
}

describe('DecisionList edit', () => {
	it('offers Edit on an open decision only', () => {
		show([open, closed]);
		const [o, c] = rows();
		actions(o)!.click();
		flushSync();
		expect(menuitem('Edit')).toBeDefined();
		expect(actions(c)).toBeNull();
		expect(button(c, 'Edit')).toBeUndefined();
	});

	it("opens on the decision's current title, decider and due date", () => {
		show([open]);
		edit(rows()[0]);
		flushSync();
		const form = editor()!;
		expect([field(form, 'Title').value, field(form, 'Decider').value, field(form, 'Due').value]).toEqual([
			open.title, open.decider, open.due
		]);
	});

	describe('focus', () => {
		it('opening the editor focuses its title', async () => {
			show([open]);
			edit(rows()[0]);
			flushSync();
			await vi.waitFor(() => {
				flushSync();
				expect(document.activeElement).toBe(field(editor()!, 'Title'));
			});
		});

		it('a successful save returns focus to the Actions trigger', async () => {
			net.replies.reply = () => net.replies.ok({ ...open, title: 'Pick the queue', version: 2 });
			show([open]);
			edit(rows()[0]);
			flushSync();
			enter(field(editor()!, 'Title'), 'Pick the queue');
			button(editor()!, 'Save')!.click();
			await vi.waitFor(() => {
				flushSync();
				expect(editor()).toBeNull();
				expect(document.activeElement).toBe(actions(rows()[0]));
			});
		});

		it('an unchanged save returns focus to the Actions trigger', async () => {
			show([open]);
			edit(rows()[0]);
			flushSync();
			button(editor()!, 'Save')!.click();
			await vi.waitFor(() => {
				flushSync();
				expect(editor()).toBeNull();
				expect(document.activeElement).toBe(actions(rows()[0]));
			});
		});

		it('a save that lands after the user moved to another control in the row leaves focus there', async () => {
			let release!: () => void;
			net.replies.gate = new Promise<void>((r) => (release = r));
			net.replies.reply = () => net.replies.ok({ ...open, title: 'Pick the queue', version: 2 });
			show([open]);
			edit(rows()[0]);
			flushSync();
			enter(field(editor()!, 'Title'), 'Pick the queue');
			button(editor()!, 'Save')!.click();
			await vi.waitFor(() => expect(net.sent).toHaveLength(1));
			// Another control in the same row: remember() takes it, and it is outside the editor, so focus is not free.
			const comment = discussion(rows()[0]);
			comment.focus();
			release();
			await vi.waitFor(() => {
				flushSync();
				expect(editor()).toBeNull();
			});
			await new Promise((r) => setTimeout(r, 20));
			expect(document.activeElement).toBe(comment);
		});

		it('a failed save returns focus to the first field', async () => {
			net.replies.reply = () => ({ status: 500, body: { outcome: 'error', code: 'internal', message: 'boom' } });
			show([open]);
			edit(rows()[0]);
			flushSync();
			enter(field(editor()!, 'Title'), 'Pick the queue');
			const save = button(editor()!, 'Save')!;
			save.focus();
			save.click();
			await vi.waitFor(() => {
				flushSync();
				expect(editAlert()?.textContent).toContain('boom');
				expect(document.activeElement).toBe(field(editor()!, 'Title'));
			});
		});

		it('Cancel returns focus to the Actions trigger', async () => {
			show([open]);
			edit(rows()[0]);
			flushSync();
			button(editor()!, 'Cancel')!.click();
			await vi.waitFor(() => {
				flushSync();
				expect(editor()).toBeNull();
				expect(document.activeElement).toBe(actions(rows()[0]));
			});
		});
	});

	it('closes without a request when nothing changed', async () => {
		show([open]);
		edit(rows()[0]);
		flushSync();
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editor()).toBeNull();
		});
		expect(net.sent).toEqual([]);
		expect(document.body.querySelector('[role="alert"]')).toBeNull();
	});

	it('refuses a blank title in place, keeps the draft and sends nothing', async () => {
		show([open]);
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), '   ');
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editAlert()?.textContent).toContain('Title and decider must not be blank.');
		});
		expect(field(editor()!, 'Title').value).toBe('   ');
		expect(net.sent).toEqual([]);
	});

	it('on a 409 reloads, keeps the changed title, takes the reloaded decider, and the retry sends only the title', async () => {
		// A colleague changed the decider (version 2) while this editor was open on version 1.
		const theirs = { ...open, decider: 'bob@example.com', version: 2 };
		net.replies.reply = (r) =>
			r.method === 'PATCH' && patches().length === 1
				? {
						status: 409,
						body: { outcome: 'error', code: 'conflict', message: 'version conflict: current is 2', current: 2 }
					}
				: net.replies.ok({ ...theirs, title: 'Pick the queue', version: 3 });
		show([open]);
		reloaded = [theirs];
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), 'Pick the queue');
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editAlert()?.textContent).toContain('Someone else changed this decision');
		});
		const form = editor()!;
		expect([field(form, 'Title').value, field(form, 'Decider').value, field(form, 'Due').value]).toEqual([
			'Pick the queue', 'bob@example.com', '2026-11-30'
		]);
		button(form, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editor()).toBeNull();
		});
		expect(patches()).toEqual([
			['/api/v1/decisions/1', { version: 1, title: 'Pick the queue' }],
			['/api/v1/decisions/1', { version: 2, title: 'Pick the queue' }]
		]);
	});

	it("the Classify picker on a 409 reloads, takes the colleague's title, and the retry sends only the reversibility", async () => {
		// A colleague renamed the decision (version 2) after this list loaded version 1.
		const theirs = { ...open, title: 'Pick a broker', version: 2 };
		net.replies.reply = (r) =>
			r.method === 'PATCH' && patches().length === 1
				? {
						status: 409,
						body: { outcome: 'error', code: 'conflict', message: 'version conflict: current is 2', current: 2 }
					}
				: net.replies.ok({ ...theirs, reversibility: 'two_way', version: 3 });
		show([open]);
		reloaded = [theirs];
		button(rows()[0], 'Classify')!.click();
		flushSync();
		menuitem('Two-way')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editAlert()?.textContent).toContain('Someone else changed this decision');
		});
		// The editor opens on the merge: the reloaded title, the chosen classification.
		const form = editor()!;
		expect(field(form, 'Title').value).toBe('Pick a broker');
		expect(form.querySelector<HTMLSelectElement>('select[name="reversibility"]')!.value).toBe('two_way');
		button(form, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editor()).toBeNull();
		});
		expect(patches()).toEqual([
			['/api/v1/decisions/1', { version: 1, reversibility: 'two_way' }],
			['/api/v1/decisions/1', { version: 2, reversibility: 'two_way' }]
		]);
	});

	it('after Classify, focus goes from the removed chip to the Actions trigger', async () => {
		const classified = { ...open, reversibility: 'one_way' as const, version: 2 };
		net.replies.reply = () => net.replies.ok(classified);
		show([open]);
		reloaded = [classified];
		button(rows()[0], 'Classify')!.click();
		flushSync();
		menuitem('One-way')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(actions(rows()[0]));
		});
		expect(rows()[0].querySelector('.chip')?.textContent).toBe('One-way');
		expect(editor()).toBeNull();
	});

	it("a classify that lands after focus moved to another row leaves that row's focus and pin alone", async () => {
		// Row B goes to the folded rest group on the reload; only its remembered focus keeps it in sight.
		const b = decision({ ...open, id: 3, title: 'Pick a lock', reversibility: 'two_way' });
		const accepted = { ...b, check: { kind: 'accepted' as const, commit: 'abcdef0123' }, version: 2 };
		let release!: () => void;
		net.replies.gate = new Promise<void>((r) => (release = r));
		net.replies.reply = () => net.replies.ok({ ...open, reversibility: 'one_way', version: 2 });
		const local = $state({
			decisions: [open, b] as RequiredDecision[],
			rest: (d: RequiredDecision) => d.check.kind === 'accepted',
			onchanged: async () => {
				local.decisions = [{ ...open, reversibility: 'one_way', version: 2 }, accepted];
			}
		});
		app = mount(DecisionList, { target: document.body, props: local });
		flushSync();
		button(rows()[0], 'Classify')!.click();
		flushSync();
		menuitem('One-way')!.click();
		await vi.waitFor(() => expect(patches().length).toBe(1));
		const other = discussion(rows()[1]);
		other.focus();
		flushSync();
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(rows()[0].querySelector('.chip')?.textContent).toBe('One-way');
		});
		expect(rows().length).toBe(2);
		expect(document.activeElement).toBe(discussion(rows()[1]));
	});

	it("after Use the record's value, focus goes from the removed button to the Actions trigger", async () => {
		const linked = {
			...open,
			reversibility: 'one_way' as const,
			record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
			check: { kind: 'accepted' as const, commit: 'abcdef0123', reversibility: 'two_way' as const }
		};
		const adopted = { ...linked, reversibility: 'two_way' as const, version: 2 };
		net.replies.reply = () => net.replies.ok(adopted);
		show([linked]);
		reloaded = [adopted];
		const use = rows()[0].querySelector<HTMLButtonElement>('[data-field="use-record"]')!;
		use.focus();
		use.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(actions(rows()[0]));
		});
		expect(rows()[0].querySelector('[data-field="use-record"]')).toBeNull();
	});

	it('disables set date while an inline classify is being sent', async () => {
		let release!: () => void;
		net.replies.gate = new Promise<void>((r) => (release = r));
		const undated = { ...open, due: null };
		net.replies.reply = () => net.replies.ok({ ...undated, reversibility: 'one_way', version: 2 });
		show([undated]);
		reloaded = [{ ...undated, reversibility: 'one_way', version: 2 }];
		const setDate = () => rows()[0].querySelector<HTMLButtonElement>('[data-field="set-due"]')!;
		expect(setDate().disabled).toBe(false);
		button(rows()[0], 'Classify')!.click();
		flushSync();
		menuitem('One-way')!.click();
		flushSync();
		expect(setDate().disabled).toBe(true);
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(setDate().disabled).toBe(false);
		});
	});

	it('refuses a classification change while a close form is open', () => {
		const linked = {
			...open,
			reversibility: null,
			record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
			check: { kind: 'accepted' as const, commit: 'abcdef0123', reversibility: 'one_way' as const }
		};
		show([linked]);
		actions(rows()[0])!.click();
		flushSync();
		menuitem('Decide')!.click();
		flushSync();
		button(rows()[0], 'Classify')!.click();
		flushSync();
		expect([menuitem('One-way')!.disabled, menuitem('Two-way')!.disabled]).toEqual([true, true]);
		edit(rows()[0]);
		expect(editor()!.querySelector<HTMLSelectElement>('select[name="reversibility"]')!.disabled).toBe(true);
		const mismatched = { ...linked, reversibility: 'two_way' as const };
		props.decisions = [mismatched];
		flushSync();
		expect(rows()[0].querySelector<HTMLButtonElement>('[data-field="use-record"]')!.disabled).toBe(true);
		expect(net.sent).toEqual([]);
	});

	it('changes and clears the reversibility in the editor, sending only it', async () => {
		const oneWay = { ...open, reversibility: 'one_way' as const };
		net.replies.reply = () => net.replies.ok({ ...oneWay, reversibility: null, version: 2 });
		show([oneWay]);
		edit(rows()[0]);
		const select = editor()!.querySelector<HTMLSelectElement>('select[name="reversibility"]')!;
		expect(select.value).toBe('one_way');
		expect([...select.options].map((o) => [o.value, o.textContent])).toEqual([
			['', 'Unset'],
			['one_way', 'One-way'],
			['two_way', 'Two-way']
		]);
		select.value = '';
		select.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editor()).toBeNull();
		});
		expect(patches()).toEqual([['/api/v1/decisions/1', { version: 1, reversibility: null }]]);
	});

	it('says so on the closed row when a colleague closed the decision under the editor', async () => {
		const theirs = { ...open, version: 2, closed: closed.closed };
		net.replies.reply = () => ({
			status: 409,
			body: { outcome: 'error', code: 'conflict', message: 'version conflict: current is 2', current: 2 }
		});
		show([open]);
		reloaded = [theirs];
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), 'Pick the queue');
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editor()).toBeNull();
			expect(alert()?.textContent).toContain('Someone else closed this decision; your edit was not saved.');
		});
		expect(rows()[0].textContent).toContain('Closed by');
	});

	it('Cancel closes the form and clears the row\'s error', async () => {
		show([open]);
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), '   ');
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editAlert()).not.toBeNull();
		});
		button(editor()!, 'Cancel')!.click();
		flushSync();
		expect(editor()).toBeNull();
		expect(alert()).toBeNull();
	});

	it('a new edit starts without the error of an editor that closed unchanged', async () => {
		show([open]);
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), '   ');
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editAlert()).not.toBeNull();
		});
		// Back to the opening title: the save is unchanged, so the editor closes without a request.
		enter(field(editor()!, 'Title'), open.title);
		button(editor()!, 'Save')!.click();
		flushSync();
		expect(editor()).toBeNull();
		edit(rows()[0]);
		flushSync();
		expect(alert()).toBeNull();
		expect(net.sent).toEqual([]);
	});

	it("keeps the editor's error apart from the thread's", async () => {
		net.replies.reply = (r) =>
			r.method === 'GET'
				? { status: 500, body: { outcome: 'error', code: 'internal', message: 'thread failed' } }
				: net.replies.unexpected(r);
		show([open]);
		discussion(rows()[0]).click();
		await vi.waitFor(() => {
			flushSync();
			expect(alert()?.textContent).toContain('thread failed');
		});
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), '   ');
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editAlert()?.textContent).toContain('Title and decider must not be blank.');
		});
		expect(editAlert()?.textContent).not.toContain('thread failed');
		// The thread's error stays at the top of the row, outside the editor.
		const all = [...rows()[0].querySelectorAll('[role="alert"]')].map((a) => a.textContent);
		expect(all).toEqual(['thread failed', 'Title and decider must not be blank.']);
	});

	it('drops the draft and error of a decision that left the list', () => {
		show([open]);
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), '   ');
		button(editor()!, 'Save')!.click();
		flushSync();
		props.decisions = [];
		flushSync();
		props.decisions = [open];
		flushSync();
		expect(editor()).toBeNull();
		expect(alert()).toBeNull();
		expect(actions(rows()[0])).not.toBeNull();
	});

	it('accepts a decider that is not an email and saves it', async () => {
		net.replies.reply = () => net.replies.ok({ ...open, decider: 'Team Lead', version: 2 });
		show([open]);
		edit(rows()[0]);
		flushSync();
		const decider = field(editor()!, 'Decider');
		expect(decider.type).toBe('text');
		expect(decider.required).toBe(true);
		enter(decider, 'Team Lead');
		expect(decider.checkValidity()).toBe(true);
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editor()).toBeNull();
		});
		expect(patches()).toEqual([['/api/v1/decisions/1', { version: 1, decider: 'Team Lead' }]]);
		expect(alert()).toBeNull();
	});

	describe('the first save after the list changed under the editor', () => {
		it('advances the base when nobody changed these fields', async () => {
			net.replies.reply = () => net.replies.ok({ ...open, title: 'Pick the queue', version: 3 });
			show([open]);
			edit(rows()[0]);
			flushSync();
			// A scheduled check moved only the version.
			props.decisions = [{ ...open, version: 2 }];
			flushSync();
			enter(field(editor()!, 'Title'), 'Pick the queue');
			button(editor()!, 'Save')!.click();
			await vi.waitFor(() => {
				flushSync();
				expect(editor()).toBeNull();
			});
			expect(patches()).toEqual([['/api/v1/decisions/1', { version: 2, title: 'Pick the queue' }]]);
			expect(alert()).toBeNull();
		});

		it("keeps the old base when a colleague changed a field, so the save conflicts instead of overwriting", async () => {
			net.replies.reply = () => ({
				status: 409,
				body: { outcome: 'error', code: 'conflict', message: 'version conflict: current is 2', current: 2 }
			});
			show([open]);
			edit(rows()[0]);
			flushSync();
			props.decisions = [{ ...open, title: 'Pick a broker', version: 2 }];
			reloaded = props.decisions;
			flushSync();
			enter(field(editor()!, 'Title'), 'Pick the queue');
			button(editor()!, 'Save')!.click();
			await vi.waitFor(() => {
				flushSync();
				expect(editAlert()?.textContent).toContain('Someone else changed this decision');
			});
			expect(patches()[0]).toEqual(['/api/v1/decisions/1', { version: 1, title: 'Pick the queue' }]);
		});
	});

	describe('a reload under an open editor', () => {
		it('keeps the typed value, the old decider, the focus and the open discussion', async () => {
			net.replies.reply = () => net.replies.ok([]);
			show([open]);
			discussion(rows()[0]).click();
			// Opening loads the thread; let that request finish so it is not mistaken for a reload's.
			await vi.waitFor(() => expect(net.sent).toHaveLength(1));
			edit(rows()[0]);
			flushSync();
			const title = field(editor()!, 'Title');
			title.focus();
			enter(title, 'Pick the queue');
			props.decisions = [{ ...open, decider: 'bob@example.com', version: 2 }];
			flushSync();
			expect(field(editor()!, 'Title')).toBe(title);
			expect(title.value).toBe('Pick the queue');
			expect(field(editor()!, 'Decider').value).toBe('dan@example.com');
			expect(document.activeElement).toBe(title);
			expect(discussion(rows()[0]).getAttribute('aria-expanded')).toBe('true');
		});

		it('keeps the value and the focus across the background reload and the clock', () => {
			vi.useFakeTimers();
			const checking: RequiredDecision = {
				...open,
				record: { record_id: 'r1', repo: 'org/repo', path: 'adr/1.md' },
				checked_at: '2026-10-27T00:00:00Z'
			};
			show([checking]);
			edit(rows()[0]);
			flushSync();
			const title = field(editor()!, 'Title');
			title.focus();
			enter(title, 'Pick the queue');
			vi.advanceTimersByTime(120_000);
			flushSync();
			expect(field(editor()!, 'Title')).toBe(title);
			expect(title.value).toBe('Pick the queue');
			expect(document.activeElement).toBe(title);
		});
	});

	it('shows a non-conflict failure, keeps the draft, and neither retries nor reloads', async () => {
		net.replies.reply = () => ({
			status: 400,
			body: { outcome: 'error', code: 'validation', message: 'decision is closed' }
		});
		show([open]);
		edit(rows()[0]);
		flushSync();
		enter(field(editor()!, 'Title'), 'Pick the queue');
		button(editor()!, 'Save')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(editAlert()?.textContent).toContain('decision is closed');
		});
		expect(field(editor()!, 'Title').value).toBe('Pick the queue');
		expect(patches()).toHaveLength(1);
		expect(net.sent.filter((r) => r.method === 'GET')).toEqual([]);
	});
});

describe('DecisionList row', () => {
	it('closes through the Actions menu: Withdraw opens the reason form, focused, and sends the kind and reason', async () => {
		net.replies.reply = () => net.replies.ok({ ...open, closed: closed.closed, version: 2 });
		show([open]);
		const row = rows()[0];
		expect(row.querySelector('input[data-field="reason"]')).toBeNull();
		actions(row)!.click();
		flushSync();
		menuitem('Withdraw')!.click();
		flushSync();
		const reason = row.querySelector<HTMLInputElement>('form[aria-label="Withdraw decision Pick a queue"] input[data-field="reason"]')!;
		await vi.waitFor(() => expect(document.activeElement).toBe(reason));
		enter(reason, 'moot');
		reason.form!.requestSubmit();
		await vi.waitFor(() => expect(net.sent).toHaveLength(1));
		expect(net.sent[0]).toEqual({ method: 'POST', path: '/api/v1/decisions/1/close', body: { kind: 'withdrawn', reason: 'moot' } });
	});

	/** Withdraws `d` through its row's Actions menu with the reason "moot". */
	async function closeRow(row: Element) {
		actions(row)!.click();
		flushSync();
		menuitem('Withdraw')!.click();
		flushSync();
		const reason = row.querySelector<HTMLInputElement>('input[data-field="reason"]')!;
		await vi.waitFor(() => expect(document.activeElement).toBe(reason));
		enter(reason, 'moot');
		reason.form!.requestSubmit();
	}
	const other = { ...open, id: 3, title: 'Pick a cache' };

	it('after Close, focus goes to the closed row\'s Discussion toggle when the row stays', async () => {
		const shut = { ...open, closed: closed.closed, version: 2 };
		net.replies.reply = () => net.replies.ok(shut);
		show([open]);
		reloaded = [shut];
		await closeRow(rows()[0]);
		await vi.waitFor(() => {
			flushSync();
			expect(rows()[0].textContent).toContain('Closed by');
			expect(document.activeElement).toBe(discussion(rows()[0]));
		});
	});

	it('after Close, focus goes to the next row\'s Actions when the row leaves the list', async () => {
		net.replies.reply = () => net.replies.ok({ ...open, closed: closed.closed, version: 2 });
		show([open, other]);
		reloaded = [other];
		await closeRow(rows()[0]);
		await vi.waitFor(() => {
			flushSync();
			expect(rows()).toHaveLength(1);
			expect(document.activeElement).toBe(actions(rows()[0]));
		});
	});

	it('after Close, focus goes to the previous row\'s Actions when the last row leaves', async () => {
		net.replies.reply = () => net.replies.ok({ ...open, closed: closed.closed, version: 2 });
		show([other, open]);
		reloaded = [other];
		await closeRow(rows()[1]);
		await vi.waitFor(() => {
			flushSync();
			expect(rows()).toHaveLength(1);
			expect(document.activeElement).toBe(actions(rows()[0]));
		});
	});

	it('after Close, focus goes to the page heading when no row is left', async () => {
		const h1 = document.body.appendChild(document.createElement('h1'));
		net.replies.reply = () => net.replies.ok({ ...open, closed: closed.closed, version: 2 });
		show([open]);
		reloaded = [];
		await closeRow(rows()[0]);
		await vi.waitFor(() => {
			flushSync();
			expect(rows()).toHaveLength(0);
			expect(document.activeElement).toBe(h1);
		});
		expect(h1.getAttribute('tabindex')).toBe('-1');
	});

	it("after Close, focus goes to the list's own section heading, not the page's, when no row is left", async () => {
		const h1 = document.body.appendChild(document.createElement('h1'));
		const section = document.body.appendChild(document.createElement('section'));
		const h3 = section.appendChild(document.createElement('h3'));
		h3.textContent = 'Required decisions';
		net.replies.reply = () => net.replies.ok({ ...open, closed: closed.closed, version: 2 });
		props.decisions = [open];
		reloaded = [];
		app = mount(DecisionList, { target: section, props });
		flushSync();
		await closeRow(rows()[0]);
		await vi.waitFor(() => {
			flushSync();
			expect(rows()).toHaveLength(0);
			expect(document.activeElement).toBe(h3);
		});
		expect(h3.getAttribute('tabindex')).toBe('-1');
		expect(h1.hasAttribute('tabindex')).toBe(false);
	});

	describe('Cancel while the close is pending: the reload removes the Actions trigger Cancel focused', () => {
		/** Submits the close, holds its answer, presses Cancel (focus goes to Actions), then lets the answer land. */
		async function cancelDuringClose(row: Element) {
			let release!: () => void;
			net.replies.gate = new Promise<void>((r) => (release = r));
			await closeRow(row);
			await vi.waitFor(() => expect(net.sent).toHaveLength(1));
			button(row, 'Cancel')!.click();
			await vi.waitFor(() => {
				flushSync();
				expect(document.activeElement).toBe(actions(row));
			});
			release();
		}

		it("focus goes to the next row's Actions when the row leaves the list, not to <body>", async () => {
			net.replies.reply = () => net.replies.ok({ ...open, closed: closed.closed, version: 2 });
			show([open, other]);
			reloaded = [other];
			await cancelDuringClose(rows()[0]);
			await vi.waitFor(() => {
				flushSync();
				expect(rows()).toHaveLength(1);
				expect(document.activeElement).toBe(actions(rows()[0]));
			});
		});

		it("focus goes to the closed row's Discussion toggle when the row stays, not to <body>", async () => {
			const shut = { ...open, closed: closed.closed, version: 2 };
			net.replies.reply = () => net.replies.ok(shut);
			show([open]);
			reloaded = [shut];
			await cancelDuringClose(rows()[0]);
			await vi.waitFor(() => {
				flushSync();
				expect(rows()[0].textContent).toContain('Closed by');
				expect(document.activeElement).toBe(discussion(rows()[0]));
			});
		});

		it('focus the user moved to a control that survives the reload stays there', async () => {
			net.replies.reply = () => net.replies.ok({ ...open, closed: closed.closed, version: 2 });
			show([open, other]);
			reloaded = [other];
			let release!: () => void;
			net.replies.gate = new Promise<void>((r) => (release = r));
			await closeRow(rows()[0]);
			await vi.waitFor(() => expect(net.sent).toHaveLength(1));
			button(rows()[0], 'Cancel')!.click();
			await vi.waitFor(() => {
				flushSync();
				expect(document.activeElement).toBe(actions(rows()[0]));
			});
			// Then the user moves on to the other row's Discussion toggle before the answer lands.
			const elsewhere = discussion(rows()[1]);
			elsewhere.focus();
			release();
			await vi.waitFor(() => {
				flushSync();
				expect(rows()).toHaveLength(1);
			});
			await new Promise((r) => setTimeout(r, 20));
			expect(document.activeElement).toBe(elsewhere);
		});
	});

	it('Cancel on the reason form closes it and returns focus to Actions', async () => {
		show([open]);
		actions(rows()[0])!.click();
		flushSync();
		menuitem('Withdraw')!.click();
		flushSync();
		button(rows()[0], 'Cancel')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(rows()[0].querySelector('input[data-field="reason"]')).toBeNull();
			expect(document.activeElement).toBe(actions(rows()[0]));
		});
		expect(net.sent).toEqual([]);
	});

	it('shows the discussion only when expanded, and loads it then', async () => {
		net.replies.reply = () => net.replies.ok([]);
		show([open]);
		const row = rows()[0];
		expect(discussion(row).getAttribute('aria-expanded')).toBe('false');
		expect(row.querySelector('textarea')).toBeNull();
		discussion(row).click();
		flushSync();
		expect(discussion(row).getAttribute('aria-expanded')).toBe('true');
		await vi.waitFor(() => {
			flushSync();
			expect(row.textContent).toContain('No comments yet.');
		});
		expect(net.sent).toEqual([{ method: 'GET', path: '/api/v1/decisions/1/comments', body: null }]);
		expect(row.querySelector('textarea[aria-label="comment on Pick a queue"]')).not.toBeNull();
	});

	it('gives every focusable control in the row a data-field', () => {
		show([open]);
		const row = rows()[0];
		edit(row);
		discussion(row).click();
		flushSync();
		const missing = [...row.querySelectorAll<HTMLElement>('a[href], button, input, textarea')].filter(
			(el) => !el.dataset.field && !el.closest('[role="menu"]')
		);
		expect(missing.map((el) => el.outerHTML)).toEqual([]);
	});
});

describe('DecisionList groups', () => {
	/** The queue's four groups, `closed` folded, each heading a plain <h2> naming its group. */
	const heading = createRawSnippet((group: () => string, count: () => number) => ({
		render: () => `<h2 data-heading="${group()}">${group()} ${count()}</h2>`
	}));
	function grouped(list: RequiredDecision[], at: Record<number, QueueGroup>) {
		const local = $state({
			decisions: list,
			group: at,
			groupOf: (d: RequiredDecision): string => local.group[d.id],
			groups: [
				{ id: 'blocking', folded: false },
				{ id: 'open', folded: false },
				{ id: 'satisfied', folded: false },
				{ id: 'closed', folded: true }
			],
			heading,
			onchanged: async () => {}
		});
		app = mount(DecisionList, { target: document.body, props: local });
		flushSync();
		return local;
	}
	/** The group whose heading sits right before the row's list. */
	const groupOfRow = (row: Element) =>
		(row.closest('ul')!.previousElementSibling as HTMLElement | null)?.dataset.heading ?? null;
	const rowOf = (title: string) => rows().find((r) => r.querySelector('.title')?.textContent === title);
	const comment = (row: Element) => row.querySelector('textarea[data-field="comment"]') as HTMLTextAreaElement;

	it('renders each group under its heading, in the given order, and a folded group as its heading only', () => {
		const b = decision({ ...open, id: 3, title: 'Pick a lock' });
		const c = decision({ ...open, id: 4, title: 'Pick a port' });
		grouped([open, b, c], { 1: 'open', 3: 'blocking', 4: 'closed' });
		// Each heading gets the rows placed under it, folded ones counted.
		expect([...document.body.querySelectorAll('h2')].map((h) => h.textContent)).toEqual(['blocking 1', 'open 1', 'closed 1']);
		expect(groupOfRow(rowOf('Pick a lock')!)).toBe('blocking');
		expect(groupOfRow(rowOf('Pick a queue')!)).toBe('open');
		expect(rowOf('Pick a port')).toBeUndefined();
	});

	it('keeps the unsent edit, the comment draft and the focus when a decision moves from blocking to open', async () => {
		net.replies.reply = () => net.replies.ok([]);
		const local = grouped([open], { 1: 'blocking' });
		discussion(rows()[0]).click();
		await vi.waitFor(() => expect(net.sent).toHaveLength(1));
		await vi.waitFor(() => {
			flushSync();
			expect(rows()[0].textContent).toContain('No comments yet.');
		});
		comment(rows()[0]).value = 'Half a thought';
		comment(rows()[0]).dispatchEvent(new Event('input', { bubbles: true }));
		edit(rows()[0]);
		const title = field(editor()!, 'Title');
		title.focus();
		enter(title, 'Pick the queue');
		expect(groupOfRow(rows()[0])).toBe('blocking');
		// The reload: the server now puts the decision in `open`.
		local.group = { 1: 'open' };
		local.decisions = [{ ...open }];
		flushSync();
		await vi.waitFor(() => {
			flushSync();
			expect(groupOfRow(rows()[0])).toBe('open');
		});
		expect(rows()).toHaveLength(1);
		expect(field(editor()!, 'Title').value).toBe('Pick the queue');
		expect(comment(rows()[0]).value).toBe('Half a thought');
		expect(document.activeElement).toBe(field(editor()!, 'Title'));
	});

	it('keeps a decision being edited where it was shown while its new group is folded, until the form closes and focus leaves', async () => {
		const b = decision({ ...open, id: 3, title: 'Pick a lock' });
		const local = grouped([open, b], { 1: 'blocking', 3: 'blocking' });
		edit(rowOf('Pick a queue')!);
		const title = field(editor()!, 'Title');
		title.focus();
		enter(title, 'Pick the queue');
		local.group = { 1: 'closed', 3: 'blocking' };
		local.decisions = [{ ...open }, b];
		flushSync();
		expect(groupOfRow(rowOf('Pick a queue')!)).toBe('blocking');
		// The pinned row counts where it is shown, not in the folded group it belongs to.
		expect(document.body.querySelector('h2[data-heading="blocking"]')?.textContent).toBe('blocking 2');
		expect(document.body.querySelector('h2[data-heading="closed"]')).toBeNull();
		expect(field(editor()!, 'Title')).toBe(title);
		expect(document.activeElement).toBe(title);
		// Cancel hands focus to the row's Actions menu, which still pins it; once focus leaves, the row folds away.
		button(editor()!, 'Cancel')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.activeElement).toBe(actions(rowOf('Pick a queue')!));
		});
		discussion(rowOf('Pick a lock')!).focus();
		await vi.waitFor(() => {
			flushSync();
			expect(rowOf('Pick a queue')).toBeUndefined();
		});
		expect(document.body.querySelector('h2[data-heading="closed"]')).not.toBeNull();
		expect(document.activeElement).toBe(discussion(rowOf('Pick a lock')!));
	});

	describe('a colleague closes a decision under its open form', () => {
		const closedBy = { at: '2026-10-02T00:00:00Z', by: 'erik@example.com', reason: 'done', kind: 'decided' as const };
		/** The reload after a colleague closed `open`: the server moves it to `closed` AND its DTO says closed. */
		function colleagueCloses(local: ReturnType<typeof grouped>, b: RequiredDecision) {
			local.group = { 1: 'closed', 3: 'blocking' };
			local.decisions = [{ ...open, version: 2, closed: closedBy }, b];
			flushSync();
		}
		const closeForm = () => document.body.querySelector('form.close');
		/** Placed in Closed: the raw heading snippet renders its count once, so the rows are counted instead. */
		function inClosed() {
			expect(rows().map(groupOfRow)).toEqual(['blocking']);
			expect(document.body.querySelector('h2[data-heading="closed"]')).not.toBeNull();
		}

		for (const form of ['edit', 'close'] as const) {
			it(`drops the hidden ${form} form, so it no longer pins the closed row in its old group`, async () => {
				const b = decision({ ...open, id: 3, title: 'Pick a lock' });
				const local = grouped([open, b], { 1: 'blocking', 3: 'blocking' });
				if (form === 'edit') {
					edit(rowOf('Pick a queue')!);
					enter(field(editor()!, 'Title'), 'Pick the queue');
				} else {
					actions(rowOf('Pick a queue')!)!.click();
					flushSync();
					menuitem('Decide')!.click();
					flushSync();
					enter(field(closeForm()!, "What was decided, and where it's written down"), 'the simple one');
				}
				// The form takes focus once it renders; then the user moves to the other row, so only the form state could
				// keep this row where it was.
				await vi.waitFor(() => {
					flushSync();
					expect(rowOf('Pick a queue')!.querySelector('form')?.contains(document.activeElement)).toBe(true);
				});
				discussion(rowOf('Pick a lock')!).focus();
				await vi.waitFor(() => {
					flushSync();
					expect(document.activeElement).toBe(discussion(rowOf('Pick a lock')!));
				});
				colleagueCloses(local, b);
				await vi.waitFor(() => {
					flushSync();
					expect(rowOf('Pick a queue')).toBeUndefined();
				});
				inClosed();
				// Unfolded, the closed row carries the notice and no form.
				local.groups = local.groups.map((g) => ({ ...g, folded: false }));
				flushSync();
				expect(groupOfRow(rowOf('Pick a queue')!)).toBe('closed');
				expect(rowOf('Pick a queue')!.querySelector('[role="alert"]')?.textContent).toBe(
					'Someone else closed this decision; your unsent changes were dropped.'
				);
				expect(rowOf('Pick a queue')!.querySelector('form')).toBeNull();
			});
		}

		it('says so on the row and moves focus from the vanished form to its Discussion toggle', async () => {
			const b = decision({ ...open, id: 3, title: 'Pick a lock' });
			const local = grouped([open, b], { 1: 'blocking', 3: 'blocking' });
			edit(rowOf('Pick a queue')!);
			const title = field(editor()!, 'Title');
			title.focus();
			enter(title, 'Pick the queue');
			colleagueCloses(local, b);
			await vi.waitFor(() => {
				flushSync();
				expect(document.activeElement).toBe(discussion(rowOf('Pick a queue')!));
			});
			expect(editor()).toBeNull();
			expect(rowOf('Pick a queue')!.querySelector('[role="alert"]')?.textContent).toBe(
				'Someone else closed this decision; your unsent changes were dropped.'
			);
			// Shown where it was while it holds focus; once focus leaves, it folds into Closed.
			expect(groupOfRow(rowOf('Pick a queue')!)).toBe('blocking');
			discussion(rowOf('Pick a lock')!).focus();
			await vi.waitFor(() => {
				flushSync();
				expect(rowOf('Pick a queue')).toBeUndefined();
			});
			inClosed();
		});
	});
});

// An open blocks list whose stories stop being blocked (a reload brings the decision back with no blocks) leaves no
// toggle to close it, so the row must not keep an empty body under it either.
describe('DecisionList blocks that go away', () => {
	it('drops the body once the decision blocks nothing', () => {
		const state = $state({
			decisions: [open],
			blocks: new Map([[1, [{ key: 'ABC-31', title: 'Tier page' }]]]) as ReadonlyMap<number, BlockedStory[]>,
			onchanged: async () => {}
		});
		app = mount(DecisionList, { target: document.body, props: state });
		flushSync();
		document.body.querySelector<HTMLButtonElement>('button[data-field="blocks"]')!.click();
		flushSync();
		expect(document.body.querySelector('.decisions > li .body .blocked')).not.toBeNull();
		state.blocks = new Map();
		flushSync();
		expect(document.body.querySelector('.decisions > li .body')).toBeNull();
	});
});
