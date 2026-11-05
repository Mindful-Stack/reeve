// @vitest-environment jsdom
import { mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Glyph from './Glyph.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('Glyph', () => {
	it('shows the glyph to the eye and the level to a screen reader', () => {
		app = mount(Glyph, { target: document.body, props: { level: 'epic' } });
		const shown = document.body.querySelector('[aria-hidden="true"]');
		expect(shown?.textContent).toBe('■');
		expect(document.body.textContent).toBe('■epic ');
		expect(shown?.getAttribute('title')).toBe('epic');
	});
});
