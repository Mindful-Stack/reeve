// @vitest-environment jsdom
// The promise chip: its words are in the DOM, the icon is decoration, and nothing lives only in a tooltip.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import type { EpicPromise, StoryPromise } from '#lib/api/types.ts';
import { withReadiness } from '#lib/test/fixtures.ts';
import { sumCounts } from '#lib/initiative.ts';
import BacklogList from './BacklogList.svelte';
import EpicCard from './EpicCard.svelte';
import PromiseChip from './PromiseChip.svelte';
import StoryCard from './StoryCard.svelte';

const fact = { on: '2026-11-20', to: 'Acme', by: 'dana@example.com', at: '2026-11-01T09:00:00Z' };
const overdue: StoryPromise = { own: fact, epic: null, on: '2026-11-20', holder: 'own', state: 'overdue', due: null, days: 3 };

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('PromiseChip', () => {
	it('shows its text in the DOM with an aria-hidden icon and no title', () => {
		app = mount(PromiseChip, { target: document.body, props: { promise: overdue } });
		const chip = document.body.querySelector('.promise-chip')!;
		expect(chip.textContent?.trim()).toBe('Overdue 3 d · 20 Nov');
		expect(chip.classList.contains('tone-warn')).toBe(true);
		// A bare .warn or .muted would pick up the global classes of that name.
		expect([...chip.classList].filter((c) => c !== 'promise-chip' && !c.startsWith('svelte-'))).toEqual(['tone-warn']);
		const svg = chip.querySelector('svg')!;
		expect(svg.getAttribute('aria-hidden')).toBe('true');
		expect(document.body.querySelectorAll('[title]')).toHaveLength(0);
	});

	it('shows an epic promise, with no icon once past', () => {
		const past: EpicPromise = { fact, due: null, past: true };
		app = mount(PromiseChip, { target: document.body, props: { promise: past } });
		const chip = document.body.querySelector('.promise-chip')!;
		expect(chip.textContent?.trim()).toBe('Past 20 Nov');
		expect(chip.querySelector('svg')).toBeNull();
		expect(chip.classList.contains('tone-muted')).toBe(true);
		expect(chip.classList.contains('muted')).toBe(false);
	});
});

describe('PromiseChip CSS', () => {
	// The longest text ("Promised 20 Nov · this sprint · via GRA-608", ~250 px) is wider than a board card (~222 px): the
	// chip must wrap inside its container, never overflow it, and never cut the state off with an ellipsis.
	const css = readFileSync(resolve(import.meta.dirname, 'PromiseChip.svelte'), 'utf8');
	const rule = /\.promise-chip \{([^}]*)\}/.exec(css)![1].replace(/\/\*[\s\S]*?\*\//g, '');
	it('wraps within its container', () => {
		expect(rule).toMatch(/max-width:\s*100%;/);
		expect(rule).toMatch(/white-space:\s*normal;/);
		expect(rule).not.toMatch(/nowrap|text-overflow|ellipsis/);
		expect(rule).toMatch(/border-radius:\s*10px;/);
	});
});

describe('hosts', () => {
	it('the board card shows the chip in .meta; a story without a promise shows none', () => {
		app = mount(StoryCard, { target: document.body, props: { item: withReadiness({ key: 'ABC-10', promise: overdue }), href: '?story=ABC-10' } });
		expect(document.body.querySelector('[data-testid="card-ABC-10"] .meta .promise-chip')?.textContent).toContain('Overdue 3 d');
		unmount(app);
		app = mount(StoryCard, { target: document.body, props: { item: withReadiness({ key: 'ABC-11' }), href: '?story=ABC-11' } });
		expect(document.body.querySelector('.promise-chip')).toBeNull();
	});

	it('a backlog row shows the chip', () => {
		const rows = [withReadiness({ key: 'ABC-1' }), withReadiness({ key: 'ABC-2', promise: overdue })];
		app = mount(BacklogList, {
			target: document.body,
			props: { rows, full: rows, epicTitle: new Map(), href: (k: string) => `?story=${k}`, onrank: () => {}, onmove: () => {}, onepic: () => {} }
		});
		const lis = [...document.body.querySelectorAll('li[data-key]')];
		expect(lis[0].querySelector('.promise-chip')).toBeNull();
		expect(lis[1].querySelector('.promise-chip')?.textContent).toContain('Overdue 3 d · 20 Nov');
		// After the title, as on the epic page's rows.
		expect(lis[1].querySelector('a.title')?.nextElementSibling?.classList.contains('promise-chip')).toBe(true);
	});

	it('an epic card shows its promise chip', () => {
		const promise: EpicPromise = { fact, due: 'this_sprint', past: false };
		app = mount(EpicCard, {
			target: document.body,
			props: { epic: { key: 'ABC-7', title: 'Checkout', urgent_mark: null, urgent_sources: [], promise }, counts: sumCounts([]), waiting: null }
		});
		expect(document.body.querySelector('.epic-card .promise-chip')?.textContent).toBe('Promised 20 Nov · this sprint');
	});
});
