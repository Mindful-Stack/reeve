// @vitest-environment jsdom
// web/src/lib/components/DueShortcuts.svelte.test.ts: the decision Due shortcuts read the sprints themselves and offer
// only ends from the server's today on; a click is an edit to the dialog around it; a failed read shows nothing.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import type { Sprints } from '#lib/api/types.ts';
import { sprint, sprintView } from '#lib/test/fixtures.ts';
import { settle } from '#lib/test/net.ts';
import DueShortcuts from './DueShortcuts.svelte';
import Harness from './DueShortcutsHarness.test.svelte';

const net = vi.hoisted(() => ({ sprints: null as unknown, gets: 0 }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	installFetch(({ path }) => {
		if (path !== '/api/v1/sprints') throw new Error(`unexpected request ${path}`);
		net.gets++;
		return net.sprints ?? failure('sprints are down');
	});
	return orig();
});

const sprints = (activeEnds: string, draftEnds: string | null): Sprints => ({
	active: sprintView(sprint({ ends_on: activeEnds })),
	draft: draftEnds ? sprint({ id: 2, state: 'draft', ends_on: draftEnds }) : null,
	closed: [],
	today: '2026-11-12'
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.sprints = null;
	net.gets = 0;
});
const buttons = () => [...document.body.querySelectorAll<HTMLButtonElement>('[role="group"][aria-label="Due date shortcuts"] button')];

it('offers both sprint ends once the sprints answer; a click presses the one it filled and keeps its dialog open', async () => {
	net.sprints = sprints('2026-11-17', '2026-12-01');
	const onclose = vi.fn();
	app = mount(Harness, { target: document.body, props: { onclose } });
	await vi.waitFor(() => expect(buttons().map((b) => b.textContent)).toEqual(['End of this sprint (17 Nov)', 'End of next sprint (1 Dec)']));
	buttons()[1].click();
	flushSync();
	expect(buttons().map((b) => b.getAttribute('aria-pressed'))).toEqual(['false', 'true']);
	expect(net.gets).toBe(1);
	// The click was an edit (a bubbling `change`), so a backdrop click no longer closes the dialog and its form.
	const d = document.body.querySelector('dialog')!;
	d.getBoundingClientRect = () => ({ left: 100, top: 100, right: 300, bottom: 300, width: 200, height: 200, x: 100, y: 100, toJSON() {} }) as DOMRect;
	d.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, clientX: 10, clientY: 10 }));
	d.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 10, clientY: 10 }));
	expect(onclose).not.toHaveBeenCalled();
});

it('leaves out a past end', async () => {
	net.sprints = sprints('2026-11-10', '2026-12-01');
	app = mount(DueShortcuts, { target: document.body, props: {} });
	await vi.waitFor(() => expect(buttons().map((b) => b.textContent)).toEqual(['End of next sprint (1 Dec)']));
});

it('renders nothing when the sprints cannot be read', async () => {
	app = mount(DueShortcuts, { target: document.body, props: {} });
	await vi.waitFor(() => expect(net.gets).toBe(1));
	await settle();
	flushSync();
	expect(document.body.querySelector('[role="group"]')).toBeNull();
});
