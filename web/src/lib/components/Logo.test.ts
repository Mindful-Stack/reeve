// @vitest-environment jsdom
import { mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Logo from './Logo.svelte';

// Copied from assets/brand/reeve-mark.svg, not from the component, so a drift between the two fails.
const MARK =
	'M13 9a3 3 0 0 1 3-3h10a3 3 0 0 1 3 3v19H13zM13 32h16v6H13zM13 42h16v6H13zM13 52h16v3a3 3 0 0 1-3 3H16a3 3 0 0 1-3-3zM33 24A18 18 0 0 1 51 6v16a2 2 0 0 1-2 2z';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('Logo', () => {
	it('draws the tally r from the brand source and hides it from screen readers', () => {
		app = mount(Logo, { target: document.body });
		const svg = document.body.querySelector('svg');
		expect(svg?.getAttribute('aria-hidden')).toBe('true');
		expect(svg?.getAttribute('focusable')).toBe('false');
		expect(svg?.getAttribute('viewBox')).toBe('0 0 64 64');
		expect(svg?.querySelector('path')?.getAttribute('d')).toBe(MARK);
		expect(document.body.textContent).toBe('');
	});
});
