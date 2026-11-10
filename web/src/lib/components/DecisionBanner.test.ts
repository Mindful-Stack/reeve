// @vitest-environment jsdom
import { mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { queueSummary } from '#lib/test/fixtures.ts';
import DecisionBanner from './DecisionBanner.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});
const show = (over: Parameters<typeof queueSummary>[0]) => {
	app = mount(DecisionBanner, { target: document.body, props: { summary: queueSummary(over) } });
	return document.body.querySelector('nav');
};
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, ' ').trim();

describe('DecisionBanner', () => {
	it('names the non-zero parts, hides the zero one and picks the number form', () => {
		const nav = show({ review: 1, this_sprint: 2, next_sprint: 0 });
		expect(text(nav)).toBe('⚑ 1 record to review · 2 block this sprint');
		expect([...nav!.querySelectorAll('a')].map((a) => a.getAttribute('href'))).toEqual(['/decisions#review', '/decisions#this-sprint']);
	});

	it('is a labelled nav, with the flag hidden from assistive technology', () => {
		const nav = show({ review: 2 });
		expect(nav?.getAttribute('aria-label')).toBe('Decisions needing attention');
		expect(nav?.querySelector('[aria-hidden="true"]')?.textContent).toBe('⚑');
		expect(text(nav)).toBe('⚑ 2 records to review');
	});

	it('uses the singular verb for one and links next sprint', () => {
		const nav = show({ this_sprint: 1, next_sprint: 3 });
		expect(text(nav)).toBe('⚑ 1 blocks this sprint · 3 block next sprint');
		expect(nav!.querySelectorAll('a')[1].getAttribute('href')).toBe('/decisions#next-sprint');
	});

	it('renders nothing with all three at zero', () => {
		expect(show({})).toBeNull();
		expect(document.body.textContent?.trim()).toBe('');
	});
});
