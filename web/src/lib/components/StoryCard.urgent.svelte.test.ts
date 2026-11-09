// @vitest-environment jsdom
// Urgent stories carry the chip and a ring on the board card and the backlog row; order is never touched.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { withReadiness } from '#lib/test/fixtures.ts';
import BacklogList from './BacklogList.svelte';
import StoryCard from './StoryCard.svelte';

const src = [{ item: 'epic' as const, key: 'ABC-2' }];
let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('StoryCard urgency', () => {
	it('an urgent story shows the chip in .meta and the urgent ring; a calm one shows neither', () => {
		app = mount(StoryCard, {
			target: document.body,
			props: { item: withReadiness({ key: 'ABC-10', urgent_sources: src }), href: '?story=ABC-10', epicTitle: 'Checkout' }
		});
		const card = document.body.querySelector('[data-testid="card-ABC-10"]')!;
		expect(card.classList.contains('urgent')).toBe(true);
		const chip = card.querySelector('.meta .urgent-chip')!;
		expect(chip.textContent).toContain('Urgent');
		expect(chip.getAttribute('title')).toContain('via epic ABC-2');
		unmount(app);
		app = mount(StoryCard, { target: document.body, props: { item: withReadiness({ key: 'ABC-11' }), href: '?story=ABC-11' } });
		const calm = document.body.querySelector('[data-testid="card-ABC-11"]')!;
		expect(calm.classList.contains('urgent')).toBe(false);
		expect(calm.querySelector('.urgent-chip')).toBeNull();
	});
});

describe('BacklogList urgency', () => {
	it('an urgent row carries the chip and ring in place', () => {
		const rows = [withReadiness({ key: 'ABC-1' }), withReadiness({ key: 'ABC-2', urgent_sources: src })];
		app = mount(BacklogList, {
			target: document.body,
			props: { rows, full: rows, epicTitle: new Map(), href: (k: string) => `?story=${k}`, onrank: () => {}, onmove: () => {}, onepic: () => {} }
		});
		flushSync();
		const lis = [...document.body.querySelectorAll('li[data-key]')];
		expect(lis.map((l) => (l as HTMLElement).dataset.key)).toEqual(['ABC-1', 'ABC-2']);
		expect(lis[0].querySelector('.urgent-chip')).toBeNull();
		expect(lis[1].classList.contains('urgent')).toBe(true);
		expect(lis[1].querySelector('.urgent-chip')?.textContent).toContain('Urgent');
	});
});
