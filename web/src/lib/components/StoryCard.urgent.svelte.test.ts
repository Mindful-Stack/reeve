// @vitest-environment jsdom
// Urgent stories carry the chip and a ring on the board card and the backlog row; order is never touched.
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { withReadiness } from '#lib/test/fixtures.ts';
import BacklogList from './BacklogList.svelte';
import StoryCard from './StoryCard.svelte';

// Read beside this file (a CSS import is empty under vitest).
const read = (f: string) => readFileSync(resolve(import.meta.dirname, f), 'utf8');
/** The box-shadow of the rule whose selector is exactly `selector` in `css`, or undefined. */
function shadow(css: string, selector: string): string | undefined {
	for (const [, sel, body] of css.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
		if (sel.replace(/\/\*[\s\S]*?\*\//g, '').trim() === selector) return /box-shadow:\s*([^;]+);/.exec(body)?.[1];
	}
	return undefined;
}

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

describe('urgent and fresh together', () => {
	// Both rings are box-shadows; without a combined rule the scoped urgent ring would hide the live-change ring.
	it('the card keeps the live-change ring beside the urgent ring', () => {
		const fresh = shadow(read('../../app.css'), '.card.fresh')!;
		expect(fresh).toBeDefined();
		const both = shadow(read('./StoryCard.svelte'), '.card.urgent.fresh');
		expect(both).toContain('var(--danger)');
		expect(both).toContain(fresh);
	});

	it('the backlog row shows the danger ring and the accent ring', () => {
		expect(shadow(read('../../app.css'), '.rows > li.fresh')).toContain('var(--accent)');
		const both = shadow(read('./BacklogList.svelte'), '.rows > li.urgent.fresh');
		expect(both).toContain('var(--danger)');
		expect(both).toContain('var(--accent)');
	});
});
