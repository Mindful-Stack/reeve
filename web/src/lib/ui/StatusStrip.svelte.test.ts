// @vitest-environment jsdom
// web/src/lib/ui/StatusStrip.svelte.test.ts
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import StatusStrip from './StatusStrip.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});
const open = { done: false };
const progress = { done: 1, total: 4 };

describe('StatusStrip', () => {
	it('a blocking cell links to its anchor with the warn tint', () => {
		app = mount(StatusStrip, {
			target: document.body,
			props: { discovery: open, progress, cell: { blocking: true as const, text: '1 blocking decision', detail: 'on X', anchor: '#decisions' } }
		});
		const a = document.body.querySelector('a.blocking')!;
		expect(a.getAttribute('href')).toBe('#decisions');
		expect(a.textContent).toContain('1 blocking decision');
	});
	it('exposes a Status group and a progress bar with its text', () => {
		app = mount(StatusStrip, {
			target: document.body,
			props: { discovery: open, progress, cell: { blocking: false as const, text: 'Nothing blocking', detail: null, anchor: null } }
		});
		expect(document.body.querySelector('[role="group"][aria-label="Status"]')).not.toBeNull();
		const bar = document.body.querySelector('[role="progressbar"]')!;
		expect(bar.getAttribute('aria-valuenow')).toBe('1');
		expect(bar.getAttribute('aria-valuemax')).toBe('4');
		expect(bar.getAttribute('aria-label')).toBe('Stories done');
		expect(bar.getAttribute('aria-valuetext')).toBe('1 of 4 stories done');
		expect(document.body.textContent).toContain('1 of 4 stories done');
	});
	it('says No stories yet, with no bar, at zero stories', () => {
		app = mount(StatusStrip, {
			target: document.body,
			props: { discovery: open, progress: { done: 0, total: 0 }, cell: { blocking: false as const, text: 'Nothing blocking', detail: null, anchor: null } }
		});
		expect(document.body.textContent).toContain('No stories yet');
		expect(document.body.querySelector('[role="progressbar"]')).toBeNull();
	});
	it('words the completed discovery without stray commas', () => {
		const cell = { blocking: false as const, text: 'Nothing blocking', detail: null, anchor: null };
		const cases: [{ done: boolean; by?: string; at?: string }, string][] = [
			[{ done: true }, 'Complete'],
			[{ done: true, by: 'ana' }, 'Complete by ana'],
			[{ done: true, by: 'ana', at: '1 Nov' }, 'Complete by ana, 1 Nov'],
			[{ done: true, at: '1 Nov' }, 'Complete, 1 Nov']
		];
		for (const [discovery, expected] of cases) {
			app = mount(StatusStrip, { target: document.body, props: { discovery, progress, cell } });
			expect(document.body.querySelector('.cell span:not(.label)')!.textContent).toBe(expected);
			unmount(app);
			app = null;
			document.body.innerHTML = '';
		}
	});
	it('a non-blocking cell is plain text', () => {
		app = mount(StatusStrip, {
			target: document.body,
			props: { discovery: open, progress, cell: { blocking: false as const, text: 'Nothing blocking', detail: null, anchor: null } }
		});
		expect(document.body.querySelector('a')).toBeNull();
		expect(document.body.textContent).toContain('Nothing blocking');
	});
	it('offers Mark discovery complete only when open and given the callback', () => {
		const cell = { blocking: false as const, text: 'Nothing blocking', detail: null, anchor: null };
		const cb = vi.fn();
		app = mount(StatusStrip, { target: document.body, props: { discovery: open, progress, cell, onmarkdiscovery: cb } });
		const btn = document.body.querySelector('button')!;
		btn.click();
		flushSync();
		expect(cb).toHaveBeenCalledOnce();
		unmount(app);
		document.body.innerHTML = '';
		app = mount(StatusStrip, { target: document.body, props: { discovery: { done: true, by: 'ana' }, progress, cell, onmarkdiscovery: cb } });
		expect(document.body.querySelector('button')).toBeNull();
		unmount(app);
		app = mount(StatusStrip, { target: document.body, props: { discovery: open, progress, cell } });
		expect(document.body.querySelector('button')).toBeNull();
	});
});
