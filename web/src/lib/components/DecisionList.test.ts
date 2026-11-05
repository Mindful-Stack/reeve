// @vitest-environment jsdom
// The decision list links a proposed record's PR only when the url is http(s): a stored `javascript:` url is plain text.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BlockedStory, RequiredDecision } from '#lib/api/types.ts';
import { decision as decisionDto } from '#lib/test/fixtures.ts';
import DecisionList from './DecisionList.svelte';

const net = vi.hoisted(() => ({ calls: [] as { method: string; path: string; body: unknown }[] }));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(({ method, path, body }) => {
		net.calls.push({ method, path, body });
		if (method === 'GET' && path.endsWith('/comments')) return [];
		if (method === 'POST' && path.endsWith('/close')) return { ...decisionDto({ id: 1 }) };
		if (method === 'PATCH') return { ...decisionDto({ id: 1 }), version: 2 };
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

function decision(pr_url: string): RequiredDecision {
	return decisionDto({
		id: 1,
		on: 'RV-1',
		title: 'Pick a queue',
		decider: 'dan@example.com',
		created_at: '2026-10-01T00:00:00Z',
		check: { kind: 'proposed_pr_open' },
		record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
		pr_url
	});
}

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

function render(pr_url: string): HTMLElement {
	app = mount(DecisionList, {
		target: document.body,
		props: { decisions: [decision(pr_url)], onchanged: async () => {} }
	});
	flushSync();
	return document.body.querySelector('.check') as HTMLElement;
}

describe('DecisionList PR link', () => {
	it('links an https PR url', () => {
		const check = render('https://github.com/x/y/pull/1');
		expect(check.querySelector('a')?.getAttribute('href')).toBe('https://github.com/x/y/pull/1');
	});

	it('renders an unsafe PR url as plain text, with no anchor', () => {
		for (const bad of ['javascript:alert(1)', 'data:text/html,<b>x</b>', 'not a url']) {
			const check = render(bad);
			expect(check.querySelector('a')).toBeNull();
			expect(check.textContent?.trim()).not.toBe('');
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});
});

describe('DecisionList check time', () => {
	function withCheck(checked_at: string | null, record = true): RequiredDecision {
		return {
			...decision('https://github.com/x/y/pull/1'),
			check: checked_at ? { kind: 'accepted', commit: 'abcdef0123' } : { kind: 'unchecked' },
			checked_at,
			record: record ? { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' } : null
		};
	}
	// The record line lives in the opened panel (the row stays one line), so each check opens it first.
	function mountOne(d: RequiredDecision): HTMLElement {
		app = mount(DecisionList, { target: document.body, props: { decisions: [d], onchanged: async () => {} } });
		flushSync();
		document.body.querySelector<HTMLButtonElement>('button[data-field="discussion"]')!.click();
		flushSync();
		return document.body;
	}

	const NOW = Date.parse('2026-10-27T12:00:00Z');
	// The hover text is the viewer's locale and time zone; both are pinned so the expected text can be written out.
	const toLocaleString = Date.prototype.toLocaleString;
	beforeEach(() => {
		vi.useFakeTimers();
		vi.setSystemTime(NOW);
		vi.spyOn(Date.prototype, 'toLocaleString').mockImplementation(function (this: Date) {
			return toLocaleString.call(this, 'en-GB', { timeZone: 'UTC' });
		});
	});
	afterEach(() => {
		net.calls.length = 0;
		vi.useRealTimers();
		vi.restoreAllMocks();
	});

	function checkedText(at: string | null): string | undefined {
		return mountOne(withCheck(at)).querySelector('.checked')?.textContent?.trim();
	}

	it('says how long ago a linked decision was checked, with the exact time on hover', () => {
		const at = new Date(NOW - 3 * 60_000 - 1_000).toISOString();
		const el = mountOne(withCheck(at)).querySelector('.checked');
		expect(el?.tagName).toBe('TIME');
		expect(el?.textContent?.trim()).toBe('checked 3m ago');
		expect(el?.getAttribute('datetime')).toBe(at);
		expect(el?.getAttribute('title')).toBe('27/10/2026, 11:56:59');
	});

	it('counts hours and days', () => {
		expect(checkedText(new Date(NOW - 5 * 3_600_000).toISOString())).toBe('checked 5h ago');
	});

	it('counts days', () => {
		expect(checkedText(new Date(NOW - 2 * 86_400_000).toISOString())).toBe('checked 2d ago');
	});

	it('shows a question mark for an unparseable date', () => {
		expect(checkedText('not a date')).toBe('checked ? ago');
	});

	it('keeps counting on an idle page, without any prop change', () => {
		const at = new Date(NOW - 60_000).toISOString();
		const el = mountOne(withCheck(at)).querySelector('.checked');
		expect(el?.textContent?.trim()).toBe('checked 1m ago');
		vi.advanceTimersByTime(4 * 60_000);
		flushSync();
		expect(el?.textContent?.trim()).toBe('checked 5m ago');
	});

	it("ticks an undated decision's open age in the date line with the check age, from one minute clock", () => {
		const at = new Date(NOW - 60_000).toISOString();
		const el = mountOne({ ...withCheck(at), created_at: at, due: null });
		const open = () => el.querySelector('.when .warning')?.textContent?.trim();
		expect(open()).toBe('No date · open 0 days');
		// Age shows only in the date line: no other cell carries the created age.
		expect([...el.querySelectorAll('.row [title]')].map((s) => s.getAttribute('title'))).not.toContain('27/10/2026, 11:59:00');
		vi.advanceTimersByTime(24 * 60 * 60_000);
		flushSync();
		expect(open()).toBe('No date · open 1 day');
		expect(el.querySelector('.checked')?.textContent?.trim()).toBe('checked 1d ago');
	});

	it('names each Discussion toggle by its decision', () => {
		const d = withCheck(null);
		const button = mountOne(d).querySelector('button[data-field="discussion"]')!;
		expect(button.getAttribute('aria-label')).toBe(`Discussion of ${d.title}`);
		expect(button.textContent).toBe('Discussion');
	});

	it('says nothing for a decision never checked', () => {
		expect(mountOne(withCheck(null)).querySelector('.checked')).toBeNull();
	});
});

// An unchanged scheduled check moves only `checked_at` and emits no event, so an open list refetches on its own every
// two minutes while the tab is visible, or "checked N ago" would grow on a healthy system.
describe('DecisionList refetch', () => {
	let state: 'visible' | 'hidden';
	beforeEach(() => {
		vi.useFakeTimers();
		state = 'visible';
		Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => state });
	});
	afterEach(() => {
		delete (document as unknown as Record<string, unknown>).visibilityState;
		vi.useRealTimers();
	});
	function mountWith(decisions: RequiredDecision[]) {
		const onchanged = vi.fn(async () => {});
		app = mount(DecisionList, { target: document.body, props: { decisions, onchanged } });
		flushSync();
		return onchanged;
	}
	const linked = decision('https://github.com/x/y/pull/1');

	it('reloads the list every two minutes while visible', () => {
		const onchanged = mountWith([linked]);
		vi.advanceTimersByTime(119_999);
		expect(onchanged).not.toHaveBeenCalled();
		vi.advanceTimersByTime(1);
		expect(onchanged).toHaveBeenCalledTimes(1);
		vi.advanceTimersByTime(120_000);
		expect(onchanged).toHaveBeenCalledTimes(2);
	});

	it('reloads nothing while hidden, and catches up when shown', () => {
		const onchanged = mountWith([linked]);
		state = 'hidden';
		document.dispatchEvent(new Event('visibilitychange'));
		vi.advanceTimersByTime(10 * 60_000);
		expect(onchanged).not.toHaveBeenCalled();
		state = 'visible';
		document.dispatchEvent(new Event('visibilitychange'));
		expect(onchanged).toHaveBeenCalledTimes(1);
	});

	it('reloads nothing when no open decision has a record (no check to follow)', () => {
		const unlinked = { ...linked, id: 2, record: null };
		const closed = { ...linked, id: 3, closed: { at: '2026-10-02T00:00:00Z', by: 'dan@example.com', reason: 'moot' } };
		const onchanged = mountWith([unlinked, closed]);
		vi.advanceTimersByTime(10 * 60_000);
		expect(onchanged).not.toHaveBeenCalled();
	});

	it('stops on unmount', () => {
		const onchanged = mountWith([linked]);
		unmount(app!);
		app = null;
		vi.advanceTimersByTime(10 * 60_000);
		expect(onchanged).not.toHaveBeenCalled();
	});
});

describe('DecisionList row', () => {
	function rowOf(d: Partial<RequiredDecision>): HTMLElement {
		app = mount(DecisionList, {
			target: document.body,
			props: {
				decisions: [{ ...decision('https://github.com/x/y/pull/1'), on_type: 'epic', ...d }],
				itemHref: (k: string) => `/epics/${k}`,
				onchanged: async () => {}
			}
		});
		flushSync();
		return document.body.querySelector('.decisions > li') as HTMLElement;
	}
	const dot = (row: Element) => row.querySelector('.status-dot .dot')!;

	it('is a flat list item: title, what it is on, the decider and an Actions menu, no disclosure', () => {
		const row = rowOf({});
		expect(row.tagName).toBe('LI');
		expect(row.querySelector('details, summary')).toBeNull();
		expect(row.querySelector('.title')?.textContent).toBe('Pick a queue');
		expect(row.textContent).toContain('on epic');
		expect(row.querySelector('a.key')?.getAttribute('href')).toBe('/epics/RV-1');
		expect(row.textContent).toContain('dan');
		expect(row.querySelector('button[aria-label="Actions for Pick a queue on RV-1"]')?.getAttribute('data-field')).toBe('actions');
	});

	it('marks an accepted record ready, an open unsatisfied one warn and a closed one muted, with the status word', () => {
		const accepted = rowOf({ check: { kind: 'accepted', commit: 'abcdef0123' } });
		expect(dot(accepted).classList).toContain('ready');
		expect(accepted.querySelector('.status-dot')?.textContent).toBe('accepted abcdef0');
		unmount(app!);
		document.body.innerHTML = '';
		const proposed = rowOf({});
		expect(dot(proposed).classList).toContain('warn');
		expect(proposed.querySelector('.status-dot')?.textContent).toBe('proposed, PR open');
		unmount(app!);
		document.body.innerHTML = '';
		const shut = rowOf({ closed: { at: '2026-10-02T00:00:00Z', by: 'dan@example.com', reason: 'moot' } });
		expect(dot(shut).classList).toContain('muted');
		expect(shut.querySelector('button[aria-label^="Actions for"]')).toBeNull();
	});
});


// kb/ADR-0013 R3: a one-way decision is decided by its accepted record, so its menu offers only Withdraw; a two-way
// (or unset) one offers Decide too. Each form says what its reason means and sends its kind with the reason.
describe('DecisionList close', () => {
	afterEach(() => {
		net.calls.length = 0;
	});
	function mountOne(over: Partial<RequiredDecision>) {
		app = mount(DecisionList, {
			target: document.body,
			props: { decisions: [decisionDto({ id: 1, title: 'Pricing', on: 'RV-1', ...over })], onchanged: async () => {} }
		});
		flushSync();
	}
	function menuItems(): HTMLButtonElement[] {
		(document.body.querySelector('[data-field="actions"]') as HTMLButtonElement).click();
		flushSync();
		return [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
	}
	function fill(items: HTMLButtonElement[], label: string, reason: string): HTMLFormElement {
		items.find((b) => b.textContent === label)!.click();
		flushSync();
		const form = document.body.querySelector('form.close') as HTMLFormElement;
		const input = form.querySelector('input[name="reason"]') as HTMLInputElement;
		input.value = reason;
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		return form;
	}
	async function send(form: HTMLFormElement) {
		(form.querySelector('button[type="submit"]') as HTMLButtonElement).click();
		await vi.waitFor(() => expect(net.calls.length).toBe(1));
		return net.calls[0];
	}

	it('offers only Withdraw on a one-way decision, says why, and sends withdrawn', async () => {
		mountOne({ reversibility: 'one_way' });
		const items = menuItems();
		expect(items.map((b) => b.textContent)).toEqual(['Edit', 'Withdraw']);
		const form = fill(items, 'Withdraw', 'dropped the tier');
		expect(form.querySelector('label')?.textContent).toContain("Why it's no longer needed");
		expect(form.textContent).toContain('decided by its accepted record');
		const call = await send(form);
		expect(call.path).toBe('/api/v1/decisions/1/close');
		expect(call.body).toEqual({ kind: 'withdrawn', reason: 'dropped the tier' });
	});

	it('decides a two-way decision with what was decided', async () => {
		mountOne({ reversibility: 'two_way' });
		const items = menuItems();
		expect(items.map((b) => b.textContent)).toEqual(['Edit', 'Decide', 'Withdraw']);
		const form = fill(items, 'Decide', 'Postgres');
		expect(form.querySelector('label')?.textContent?.trim()).toBe("What was decided, and where it's written down");
		expect(form.querySelector<HTMLInputElement>('input[name="reason"]')?.placeholder).toBe('docs/behaviour/orgs.md org.R7');
		// While the form is open, the menu cannot open a second close form.
		const reopened = menuItems().filter((b) => b.textContent !== 'Edit');
		expect(reopened.map((b) => [b.textContent, b.disabled])).toEqual([
			['Decide', true],
			['Withdraw', true]
		]);
		const call = await send(form);
		expect(call.body).toEqual({ kind: 'decided', reason: 'Postgres' });
	});
});

// The card: the reversibility chip and its picker, the stories a decision blocks, the record's own
// classification when it disagrees, the date line and the outcome of a close.
describe('DecisionList card', () => {
	afterEach(() => {
		net.calls.length = 0;
		vi.useRealTimers();
	});
	const FOUR: BlockedStory[] = [
		{ key: 'ABC-31', title: 'Tier page' },
		{ key: 'ABC-32', title: 'Tier pricing' },
		{ key: 'ABC-33', title: 'Tier billing' },
		{ key: 'ABC-34', title: 'Tier emails' }
	];
	function mountOne(over: Partial<RequiredDecision>, blocks?: ReadonlyMap<number, BlockedStory[]>): HTMLElement {
		app = mount(DecisionList, {
			target: document.body,
			props: {
				decisions: [decisionDto({ id: 1, title: 'Pricing', on: 'RV-1', decider: 'ella@example.com', ...over })],
				itemHref: (k: string) => `/board?story=${k}`,
				blocks,
				onchanged: async () => {}
			}
		});
		flushSync();
		return document.body.querySelector('.decisions > li') as HTMLElement;
	}
	const buttonNamed = (root: Element, name: string) =>
		[...root.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.trim() === name);
	const menuitem = (name: string) =>
		[...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((b) => b.textContent?.trim() === name);
	const patches = () => net.calls.filter((c) => c.method === 'PATCH').map((c) => [c.path, c.body]);

	it('shows a one-way blocking decision with its chip and the stories it blocks, each linking its drawer', () => {
		const row = mountOne({ reversibility: 'one_way' }, new Map([[1, FOUR]]));
		expect(row.querySelector('.chip')?.textContent?.trim()).toBe('One-way');
		const toggle = buttonNamed(row, 'Blocks 4 stories ▸')!;
		expect(toggle.getAttribute('aria-expanded')).toBe('false');
		toggle.click();
		flushSync();
		expect(toggle.getAttribute('aria-expanded')).toBe('true');
		const links = [...row.querySelectorAll<HTMLAnchorElement>('.blocked a')];
		expect(links.map((a) => [a.getAttribute('href'), a.textContent?.replace(/\s+/g, ' ').trim()])).toEqual([
			['/board?story=ABC-31', 'ABC-31 Tier page'],
			['/board?story=ABC-32', 'ABC-32 Tier pricing'],
			['/board?story=ABC-33', 'ABC-33 Tier billing'],
			['/board?story=ABC-34', 'ABC-34 Tier emails']
		]);
	});

	it('names its controls by the decision, hides the glyphs and turns the arrow when open', () => {
		vi.useFakeTimers({ toFake: ['Date'] });
		const row = mountOne(
			{
				due: null,
				reversibility: 'one_way',
				record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
				check: { kind: 'accepted', commit: 'abcdef0123', reversibility: 'two_way' }
			},
			new Map([[1, FOUR]])
		);
		const label = (field: string) => row.querySelector(`[data-field="${field}"]`)?.getAttribute('aria-label');
		expect([label('set-due'), label('use-record'), label('blocks')]).toEqual([
			'Set the due date of Pricing',
			"Use the record's value for Pricing",
			'Blocks 4 stories: Pricing'
		]);
		const arrow = () => row.querySelector('[data-field="blocks"] [aria-hidden="true"]')?.textContent;
		expect(arrow()).toBe('▸');
		row.querySelector<HTMLButtonElement>('[data-field="blocks"]')!.click();
		flushSync();
		expect(arrow()).toBe('▾');
		expect(row.querySelector('.mismatch [aria-hidden="true"]')?.textContent).toBe('⚠');
	});

	it('says in the withdraw form which stories withdrawing would unblock', () => {
		const row = mountOne({ reversibility: 'one_way' }, new Map([[1, FOUR]]));
		row.querySelector<HTMLButtonElement>('[data-field="actions"]')!.click();
		flushSync();
		menuitem('Withdraw')!.click();
		flushSync();
		const form = row.querySelector('form.close')!;
		expect(form.textContent).toContain('Withdrawing lifts the gate on 4 stories');
		for (const s of FOUR) expect(form.textContent).toContain(s.key);
	});

	it('counts one blocked story in the singular', () => {
		const row = mountOne({}, new Map([[1, FOUR.slice(0, 1)]]));
		expect(buttonNamed(row, 'Blocks 1 story ▸')).toBeDefined();
	});

	it('shows no blocks line without a blocks prop, or for a decision missing from it', () => {
		expect(mountOne({}).textContent).not.toContain('Blocks');
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		expect(mountOne({}, new Map([[2, FOUR]])).textContent).not.toContain('Blocks');
	});

	it('offers Classify on an open unset decision; choosing Two-way sends only the reversibility', async () => {
		const row = mountOne({ reversibility: null, version: 4 });
		const chip = buttonNamed(row, 'Classify')!;
		expect(chip.classList).toContain('chip');
		chip.click();
		flushSync();
		expect([...document.body.querySelectorAll('[role="menuitem"]')].map((b) => b.textContent)).toEqual(['One-way', 'Two-way']);
		menuitem('Two-way')!.click();
		await vi.waitFor(() => expect(patches().length).toBe(1));
		expect(patches()).toEqual([['/api/v1/decisions/1', { version: 4, reversibility: 'two_way' }]]);
	});

	it("warns when the record's reversibility differs, and adopts it on request", async () => {
		const row = mountOne({
			reversibility: 'one_way',
			version: 3,
			record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
			check: { kind: 'not_accepted', commit: 'abcdef0123', status: 'proposed', reversibility: 'two_way' }
		});
		expect(row.querySelector('.mismatch')?.textContent).toContain('record says two-way');
		buttonNamed(row, "Use the record's value")!.click();
		await vi.waitFor(() => expect(patches().length).toBe(1));
		expect(patches()).toEqual([['/api/v1/decisions/1', { version: 3, reversibility: 'two_way' }]]);
	});

	it('shows no warning when the record agrees or reports nothing', () => {
		const row = mountOne({
			reversibility: 'one_way',
			record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
			check: { kind: 'accepted', commit: 'abcdef0123', reversibility: null }
		});
		expect(row.querySelector('.mismatch')).toBeNull();
		expect(buttonNamed(row, "Use the record's value")).toBeUndefined();
	});

	describe('closed', () => {
		const at = '2026-10-02T00:00:00Z';
		it('says what was decided, and by whom', () => {
			const row = mountOne({ reversibility: 'two_way', closed: { at, by: 'dan@example.com', reason: 'Postgres', kind: 'decided', via: 'interactive' } });
			expect(row.querySelector('.outcome')?.textContent).toContain('Decided by dan: Postgres');
			expect(row.textContent).not.toContain('automation');
		});
		it('says why it was withdrawn', () => {
			const row = mountOne({ closed: { at, by: 'dan@example.com', reason: 'dropped', kind: 'withdrawn' } });
			expect(row.querySelector('.outcome')?.textContent).toContain('Withdrawn by dan: dropped');
		});
		it('says only closed for a legacy close without a kind', () => {
			const row = mountOne({ closed: { at, by: 'dan@example.com', reason: 'moot' } });
			expect(row.querySelector('.outcome')?.textContent).toContain('Closed by dan');
		});
		it('names an automation token as the closer', () => {
			const row = mountOne({ closed: { at, by: 'dan@example.com', reason: 'moot', kind: 'withdrawn', via: 'automation' } });
			expect(row.querySelector('.outcome')?.textContent).toContain('via an automation token');
		});
		it('shows the chip read-only, with no picker', () => {
			const row = mountOne({ reversibility: 'one_way', closed: { at, by: 'dan@example.com', reason: 'moot', kind: 'withdrawn' } });
			const chip = row.querySelector('.chip')!;
			expect(chip.tagName).toBe('SPAN');
			expect(chip.textContent?.trim()).toBe('One-way');
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
			const unset = mountOne({ reversibility: null, closed: { at, by: 'dan@example.com', reason: 'moot' } });
			expect(buttonNamed(unset, 'Classify')).toBeUndefined();
		});
	});

	it('shows who decides and an undated decision\'s age, with a set date control opening the editor at the due field', async () => {
		vi.useFakeTimers({ toFake: ['Date'] });
		vi.setSystemTime(new Date(2026, 9, 13, 12, 0));
		const row = mountOne({ due: null, created_at: new Date(2026, 9, 1, 9, 0).toISOString() });
		// The column has no header, so a screen reader hears the role with the name.
		expect(row.querySelector('.decider')?.textContent?.trim()).toBe('ella decides');
		expect(row.querySelector('.decider .visually-hidden')?.textContent?.trim()).toBe('decides');
		expect(row.querySelector('.decider')?.getAttribute('title')).toBe('ella@example.com');
		const line = row.querySelector('.when')!;
		expect(line.textContent?.replace(/\s+/g, ' ').trim()).toBe('No date · open 12 days set date');
		expect(line.querySelector('.warning')).not.toBeNull();
		buttonNamed(row, 'set date')!.click();
		flushSync();
		expect(row.querySelector('form.edit')).not.toBeNull();
		await vi.waitFor(() => expect(document.activeElement).toBe(row.querySelector('form.edit input[name="due"]')));
	});

	it('marks an overdue date line as danger, with no set date control', () => {
		vi.useFakeTimers({ toFake: ['Date'] });
		vi.setSystemTime(new Date(2026, 10, 3, 12, 0));
		const row = mountOne({ due: '2026-11-01' });
		const line = row.querySelector('.when')!;
		expect(line.textContent).toContain('Overdue 2 days (Sun 1 Nov)');
		expect(line.querySelector('.danger')).not.toBeNull();
		expect(buttonNamed(row, 'set date')).toBeUndefined();
	});
});

// One line per decision: every row has the same cells in the same order, so the list's columns line up (a missing
// value is an empty cell, never a missing one), and the record's path and check age wait in the opened panel.
describe('DecisionList columns', () => {
	const COLUMNS = ['title', 'decider', 'when', 'blocks', 'on', 'check', 'discussion', 'actions'];
	const columnsOf = (li: Element) => [...li.querySelector('.row')!.children].map((c) => c.getAttribute('data-col'));

	function mountMany(decisions: RequiredDecision[], blocks?: ReadonlyMap<number, BlockedStory[]>): HTMLElement[] {
		app = mount(DecisionList, {
			target: document.body,
			props: { decisions, blocks, itemHref: (k: string) => `/epics/${k}`, onchanged: async () => {} }
		});
		flushSync();
		return [...document.body.querySelectorAll<HTMLElement>('.decisions > li')];
	}

	it('gives an open blocking row and a closed row the same columns in the same order', () => {
		const rows = mountMany(
			[
				decisionDto({ id: 1, title: 'Pricing', on: 'RV-1', due: null }),
				decisionDto({ id: 2, title: 'Queue', on: 'RV-2', closed: { at: '2026-10-02T00:00:00Z', by: 'dan@example.com', reason: 'moot' } })
			],
			new Map([[1, [{ key: 'ABC-31', title: 'Tier page' }]]])
		);
		expect(rows.map(columnsOf)).toEqual([COLUMNS, COLUMNS]);
	});

	it('keeps the record path and the check age out of the row until the discussion opens', () => {
		const [row] = mountMany([
			decisionDto({
				id: 1,
				title: 'Pricing',
				on: 'RV-1',
				check: { kind: 'accepted', commit: 'abcdef0123' },
				checked_at: new Date().toISOString(),
				record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' }
			})
		]);
		expect(row.textContent).not.toContain('docs/adr/1.md');
		expect(row.querySelector('.checked')).toBeNull();
		row.querySelector<HTMLButtonElement>('button[data-field="discussion"]')!.click();
		flushSync();
		expect(row.querySelector('.body .record')?.textContent).toBe('kb/docs/adr/1.md');
		expect(row.querySelector('.body .checked')?.textContent?.trim()).toBe('checked 0m ago');
	});

	it('shows a record mismatch without opening anything, since it asks for an action', () => {
		const [row] = mountMany([
			decisionDto({
				id: 1,
				title: 'Pricing',
				on: 'RV-1',
				reversibility: 'one_way',
				record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
				check: { kind: 'not_accepted', commit: 'abcdef0123', status: 'proposed', reversibility: 'two_way' }
			})
		]);
		expect(row.querySelector('.body .mismatch')?.textContent).toContain('record says two-way');
	});
});
