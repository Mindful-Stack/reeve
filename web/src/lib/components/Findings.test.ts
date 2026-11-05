// @vitest-environment jsdom
import { mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Findings from './Findings.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('Findings', () => {
	it('lists each finding as advisory', () => {
		app = mount(Findings, {
			target: document.body,
			props: { findings: [{ finding: 'empty_brief' }, { finding: 'discovery_complete_without_stories' }] }
		});
		const items = [...document.body.querySelectorAll('li')].map((li) => li.textContent);
		expect(items).toEqual(['the brief is empty', 'discovery is complete and there are no stories under it']);
		expect(document.body.textContent).toContain('never block');
	});

	it('renders nothing when there are none', () => {
		app = mount(Findings, { target: document.body, props: { findings: [] } });
		expect(document.body.querySelector('section')).toBeNull();
	});
});
