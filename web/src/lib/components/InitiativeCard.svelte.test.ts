// @vitest-environment jsdom
// The planning card: "Move to" every configured horizon but the current one, the blocking-decisions line, discovery.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Initiative } from '#lib/api/types.ts';
import { decision } from '#lib/test/fixtures.ts';
import InitiativeCard from './InitiativeCard.svelte';

const ini: Initiative = { key: 'R-1', title: 'Plan', brief: '', horizon: 'Next', version: 1, progress: 0.5, urgent_mark: null, urgent_sources: [] };

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('InitiativeCard', () => {
	it('offers a move to every configured horizon but the current one', () => {
		const onmove = vi.fn();
		app = mount(InitiativeCard, { target: document.body, props: { initiative: ini, buckets: ['Now', 'Next', 'Later'], onmove } });
		document.body.querySelector<HTMLButtonElement>('button[aria-label="Actions for R-1"]')!.click();
		flushSync();
		const items = [...document.body.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
		expect(items.map((b) => b.textContent)).toEqual(['Move to Now', 'Move to Later']);
		items[1].click();
		expect(onmove).toHaveBeenCalledWith('R-1', 'Later');
	});

	it('words the blocking decisions and shows discovery open and progress', () => {
		const oldest = new Date(Date.now() - 3 * 86_400_000).toISOString();
		app = mount(InitiativeCard, {
			target: document.body,
			props: { initiative: ini, decisions: { count: 2, oldest, open: [] } }
		});
		const text = document.body.textContent!.replace(/\s+/g, ' ');
		expect(text).toContain('2 blocking decisions · oldest');
		expect(text).toContain('Discovery open');
		expect(text).toContain('50% done');
		expect(document.body.querySelector('a')!.getAttribute('href')).toBe('/initiatives/R-1');
		expect(document.body.querySelector('[role="menu"], button[aria-haspopup]')).toBeNull();
	});

	it('lists the blocking decision titles for assistive technology', () => {
		const at = new Date().toISOString();
		const open = [decision({ id: 1, title: 'Pick a queue' }), decision({ id: 2, title: 'Pick a store' })];
		app = mount(InitiativeCard, { target: document.body, props: { initiative: ini, decisions: { count: 2, oldest: at, open } } });
		const line = document.body.querySelector('.blocking')!;
		const list = document.getElementById(line.getAttribute('aria-describedby')!)!;
		expect([...list.querySelectorAll('li')].map((li) => li.textContent)).toEqual(['Pick a queue', 'Pick a store']);
	});
});
