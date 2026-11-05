// @vitest-environment jsdom
// The header goal picker: nothing loads until it opens; the dialog lists the draft's goals, then the active sprint's,
// with link counts; picking a goal starts linking and closes the dialog; ✕ ends it; + Add goal shows the form.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { goal, sprint, sprintView } from '#lib/test/fixtures.ts';
import { live } from '#lib/events.svelte.ts';
import { picker, resetPicker } from '#lib/goalPicker.svelte.ts';
import GoalPicker from './GoalPicker.svelte';

const net = vi.hoisted(() => ({ sprints: null as unknown, gets: 0, hold: null as Promise<void> | null }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(async ({ method, path }) => {
		if (method === 'GET' && path === '/api/v1/sprints') {
			net.gets++;
			return net.sprints;
		}
		if (method === 'POST' && path === '/api/v1/goals') {
			await net.hold;
			return { goal: goal({ id: 6, sprint_id: 2, outcome: 'New' }), warnings: [] };
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
	resetPicker();
	net.gets = 0;
	net.hold = null;
});

const seeded = () => ({
	draft: sprint({ id: 2, state: 'draft', opened_at: null, goals: [goal({ id: 5, sprint_id: 2, outcome: 'Ship X', links: ['ABC-6', 'ABC-9'] })] }),
	active: sprintView(sprint({ goals: [goal({ id: 1, outcome: 'Cut Y' })] })),
	closed: []
});
const button = (text: string) => [...document.body.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes(text));
const dialog = () => document.body.querySelector('dialog');

async function openPicker() {
	app = mount(GoalPicker, { target: document.body });
	flushSync();
	expect(net.gets).toBe(0); // nothing loads before the picker opens
	button('Goals')!.click();
	await vi.waitFor(() => {
		flushSync();
		expect(button('Ship X')).toBeDefined();
	});
}

describe('GoalPicker', () => {
	it('lists the next sprint’s goals, then the active sprint’s, with link counts, and offers the add form', async () => {
		net.sprints = seeded();
		await openPicker();
		expect([...dialog()!.querySelectorAll('h3')].map((h) => h.textContent)).toEqual(['Next sprint', 'Active sprint']);
		expect(button('Ship X')!.textContent).toContain('2');
		button('Add goal')!.click();
		flushSync();
		expect(document.body.querySelector('form[aria-label="New goal"]')).not.toBeNull();
	});

	it('says so when no sprint is planned', async () => {
		net.sprints = { draft: null, active: null, closed: [] };
		app = mount(GoalPicker, { target: document.body });
		button('Goals')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(dialog()?.textContent).toContain('No sprint planned yet');
		});
	});

	it('picking a goal closes the dialog and starts linking; ✕ ends it', async () => {
		net.sprints = seeded();
		await openPicker();
		button('Ship X')!.click();
		flushSync();
		expect(dialog()).toBeNull();
		expect(picker.goalId).toBe(5);
		expect(document.body.textContent).toContain('Linking to: Ship X');
		document.body.querySelector<HTMLButtonElement>('button[aria-label="Stop linking"]')!.click();
		flushSync();
		expect(picker.goalId).toBeNull();
		expect(button('Goals')).toBeDefined();
	});

	it('keeps focus on the pill after a pick and on Goals after ✕', async () => {
		net.sprints = seeded();
		await openPicker();
		button('Ship X')!.click();
		await vi.waitFor(() => expect(document.activeElement).toBe(button('Linking to')));
		document.body.querySelector<HTMLButtonElement>('button[aria-label="Stop linking"]')!.click();
		await vi.waitFor(() => expect(document.activeElement).toBe(button('Goals')));
	});

	const newGoalForm = () => document.body.querySelector('form[aria-label="New goal"]');
	const fill = (name: string, v: string) => {
		const el = newGoalForm()!.querySelector<HTMLInputElement>(`[name="${name}"]`)!;
		el.value = v;
		el.dispatchEvent(new Event('input', { bubbles: true }));
	};

	it('moves focus to + Add goal after an add', async () => {
		net.sprints = seeded();
		await openPicker();
		button('Add goal')!.click();
		flushSync();
		fill('outcome', 'New');
		fill('check', 'done');
		flushSync();
		newGoalForm()!.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelector('form[aria-label="New goal"]')).toBeNull();
		});
		await vi.waitFor(() => expect(document.activeElement).toBe(button('+ Add goal')));
	});

	// Fails if a slow add's answer, arriving after its dialog was closed, folds the form opened since.
	it('a slow add answering after a close and reopen leaves the new form alone', async () => {
		net.sprints = seeded();
		let release!: () => void;
		net.hold = new Promise((r) => (release = r));
		await openPicker();
		button('Add goal')!.click();
		flushSync();
		fill('outcome', 'First');
		fill('check', 'done');
		flushSync();
		newGoalForm()!.querySelector<HTMLButtonElement>('button[type="submit"]')!.click();
		document.body.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click();
		flushSync();
		expect(dialog()).toBeNull();
		button('Goals')!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(button('+ Add goal')).toBeDefined();
		});
		button('+ Add goal')!.click();
		flushSync();
		fill('outcome', 'Second');
		flushSync();
		const gets = net.gets;
		release();
		await vi.waitFor(() => expect(net.gets).toBeGreaterThan(gets)); // the first add's reload ran
		await new Promise((r) => setTimeout(r, 0));
		flushSync();
		expect(newGoalForm()).not.toBeNull();
		expect(newGoalForm()!.querySelector<HTMLInputElement>('[name="outcome"]')!.value).toBe('Second');
	});

	it('does not reload on a live change while closed and not linking', async () => {
		net.sprints = seeded();
		app = mount(GoalPicker, { target: document.body });
		flushSync();
		live.version++;
		flushSync();
		await new Promise((r) => setTimeout(r, 700));
		expect(net.gets).toBe(0);
	});

	it('shows Loading only while there is no error', async () => {
		net.sprints = null;
		app = mount(GoalPicker, { target: document.body });
		button('Goals')!.click();
		picker.error = 'boom';
		await vi.waitFor(() => {
			flushSync();
			expect(dialog()?.textContent).not.toContain('Loading');
		});
	});
});
