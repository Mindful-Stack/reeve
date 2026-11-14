// @vitest-environment jsdom
import { mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import type { DiscoveryEntry, QueueLane } from '#lib/api/types.ts';
import { discoveryEntry, queueSummary } from '#lib/test/fixtures.ts';
import DecisionBanner from './DecisionBanner.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});
const show = (summary: Parameters<typeof queueSummary>[0], discoveries: DiscoveryEntry[] = []) => {
	app = mount(DecisionBanner, { target: document.body, props: { queue: { summary: queueSummary(summary), discoveries } } });
	return document.body.querySelector('nav');
};
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, ' ').trim();
const found = (lane: QueueLane, n: number) => Array.from({ length: n }, (_, i) => discoveryEntry({ key: `ABC-${i + 1}`, lane }));

describe('DecisionBanner', () => {
	it('names discoveries and decisions per window with the number form, each part one link to its lane', () => {
		const nav = show({ this_sprint: 1, next_sprint: 3, review: 4 }, [...found('this_sprint', 2), ...found('next_sprint', 2), ...found('beyond', 3)]);
		expect(text(nav)).toBe('⚑ 2 in discovery and 1 decision needed this sprint · 2 in discovery and 3 decisions needed next sprint');
		expect([...nav!.querySelectorAll('a')].map((a) => a.getAttribute('href'))).toEqual(['/decisions#this-sprint', '/decisions#next-sprint']);
	});

	it('is a labelled nav, with the flag hidden from assistive technology', () => {
		const nav = show({}, found('this_sprint', 1));
		expect(nav?.getAttribute('aria-label')).toBe('Readiness needing attention');
		expect(nav?.querySelector('[aria-hidden="true"]')?.textContent).toBe('⚑');
		expect(text(nav)).toBe('⚑ 1 in discovery needed this sprint');
	});

	it('renders nothing without a window part: records to review and Now rows are not banner parts', () => {
		expect(show({ review: 2 }, found('beyond', 2))).toBeNull();
		expect(document.body.textContent?.trim()).toBe('');
	});
});
