// @vitest-environment jsdom
// The promise chip: its words are in the DOM, the icon is decoration, and nothing lives only in a tooltip.
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
		expect(chip.classList.contains('warn')).toBe(true);
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
		expect(chip.classList.contains('muted')).toBe(true);
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
