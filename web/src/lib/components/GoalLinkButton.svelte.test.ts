// @vitest-environment jsdom
// The link button an item shows while linking: hidden otherwise, "+ Link" when the picked goal does not link the key
// directly, "✓ Linked" when it does; a click links, and the reload turns it into ✓.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Sprints } from '#lib/api/types.ts';
import { goal, sprint, sprintView } from '#lib/test/fixtures.ts';
import { picker, resetPicker } from '#lib/goalPicker.svelte.ts';
import GoalLinkButton from './GoalLinkButton.svelte';

const net = vi.hoisted(() => ({ sprints: null as unknown, posts: 0, gate: null as Promise<void> | null }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	const { goal } = await import('#lib/test/fixtures.ts');
	installFetch(async ({ method, path }) => {
		if (method === 'GET' && path === '/api/v1/sprints') return net.sprints;
		if (method === 'POST' && path === '/api/v1/goals/1/links') {
			net.posts++;
			if (net.gate) await net.gate;
			return goal();
		}
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

const withLinks = (links: string[]): Sprints => ({
	draft: null,
	active: sprintView(sprint({ goals: [goal({ id: 1, outcome: 'Cut Y', links })] })),
	closed: [],
	today: '2026-11-12'
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	resetPicker();
	net.posts = 0;
	net.gate = null;
});

function linking() {
	picker.sprints = withLinks(['ABC-6']);
	picker.goalId = 1;
}
const button = () => document.body.querySelector<HTMLButtonElement>('button[aria-label^="Link "]');

describe('GoalLinkButton', () => {
	it('renders nothing when not linking', () => {
		app = mount(GoalLinkButton, { target: document.body, props: { key: 'ABC-7' } });
		expect(button()).toBeNull();
	});

	it('offers "+ Link" for a key the goal does not link, naming the key and the goal', () => {
		linking();
		app = mount(GoalLinkButton, { target: document.body, props: { key: 'ABC-7' } });
		expect(button()?.textContent?.trim()).toBe('+ Link');
		expect(button()?.getAttribute('aria-label')).toBe('Link ABC-7 to goal: Cut Y');
		expect(button()?.getAttribute('aria-pressed')).toBe('false');
	});

	it('shows "✓ Linked", pressed, for a key the goal links', () => {
		linking();
		app = mount(GoalLinkButton, { target: document.body, props: { key: 'ABC-6' } });
		expect(button()?.textContent?.trim()).toBe('✓ Linked');
		expect(button()?.getAttribute('aria-pressed')).toBe('true');
	});

	it('links on click and shows ✓ once the reload has it', async () => {
		linking();
		net.sprints = withLinks(['ABC-6', 'ABC-7']);
		app = mount(GoalLinkButton, { target: document.body, props: { key: 'ABC-7' } });
		button()!.click();
		await vi.waitFor(() => {
			flushSync();
			expect(button()?.getAttribute('aria-pressed')).toBe('true');
		});
	});

	it('is disabled only while a write is in flight and sends one POST for two clicks', async () => {
		linking();
		net.sprints = withLinks(['ABC-6', 'ABC-7']);
		let release!: () => void;
		net.gate = new Promise<void>((r) => (release = r));
		app = mount(GoalLinkButton, { target: document.body, props: { key: 'ABC-7' } });
		const b = button()!;
		expect(b.getAttribute('aria-disabled')).toBe('false');
		b.click();
		await vi.waitFor(() => expect(net.posts).toBe(1));
		flushSync();
		expect(b.getAttribute('aria-disabled')).toBe('true');
		b.click();
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(button()?.getAttribute('aria-pressed')).toBe('true');
		});
		expect(button()?.getAttribute('aria-disabled')).toBe('false');
		expect(net.posts).toBe(1);
	});
});
