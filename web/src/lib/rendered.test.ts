// @vitest-environment jsdom
// Rendered Markdown containers swap a broken or deleted attachment image for "image unavailable" (#lib/rendered.ts).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { rendered } from './rendered.ts';
import { renderMarkdown } from './markdown.ts';

const id = '0f8b2c3e-1a2b-4c3d-8e9f-0a1b2c3d4e5f';
let detach: () => void = () => {};
afterEach(() => {
	detach();
	document.body.innerHTML = '';
	vi.unstubAllGlobals();
});

function mountBox(md: string): HTMLElement {
	const box = document.createElement('div');
	box.innerHTML = renderMarkdown(md);
	document.body.append(box);
	detach = rendered()(box) as () => void;
	return box;
}

describe('rendered', () => {
	it('turns an image that is already shown into a chip when it is deleted', () => {
		const box = mountBox(`Before ![a](/api/v1/attachments/${id}) after ![b](https://example.com/b.png)`);
		window.dispatchEvent(new CustomEvent('reeve:attachment-removed', { detail: { id } }));
		expect(box.querySelector(`img[src$="${id}"]`)).toBeNull();
		expect(box.querySelector('.image-unavailable')?.textContent).toBe('image unavailable');
		// Its wrapping link goes with it; another image stays.
		expect(box.querySelector('a')).toBeNull();
		expect(box.querySelector('img[src="https://example.com/b.png"]')).not.toBeNull();
	});

	it('hears a failed load, which does not bubble, through capture', () => {
		const box = mountBox(`![a](/api/v1/attachments/${id})`);
		box.querySelector('img')!.dispatchEvent(new Event('error'));
		expect(box.querySelector('.image-unavailable')).not.toBeNull();
	});

	it('ignores an external image that fails', () => {
		const box = mountBox('![b](https://example.com/b.png)');
		box.querySelector('img')!.dispatchEvent(new Event('error'));
		expect(box.querySelector('.image-unavailable')).toBeNull();
	});
});

describe('rendered tables', () => {
	const table = '| a | b |\n|---|---|\n| 1 | 2 |';
	// jsdom lays nothing out: a wrapper's widths are set by hand, and the stub hands back what a resize would call.
	let resized: () => void = () => {};
	function stubResize(): void {
		vi.stubGlobal(
			'ResizeObserver',
			class {
				constructor(cb: () => void) {
					resized = cb;
				}
				observe() {}
				disconnect() {}
			}
		);
	}
	function widths(wrap: Element, scroll: number, client: number): void {
		Object.defineProperty(wrap, 'scrollWidth', { configurable: true, value: scroll });
		Object.defineProperty(wrap, 'clientWidth', { configurable: true, value: client });
	}
	const stop = (wrap: Element) => [wrap.getAttribute('tabindex'), wrap.getAttribute('role'), wrap.getAttribute('aria-label')];

	it('leaves the wrapper of a table that fits out of the tab order', () => {
		stubResize();
		const box = mountBox(table);
		expect(stop(box.querySelector('.table-scroll')!)).toEqual([null, null, null]);
	});

	it('makes a wrapper that scrolls sideways a named tab stop, and takes that back when the table fits again', () => {
		stubResize();
		const box = mountBox(`${table}\n\ntext\n\n${table}`);
		const [wide, narrow] = box.querySelectorAll('.table-scroll');
		widths(wide, 900, 320);
		widths(narrow, 200, 320);
		resized();
		expect(stop(wide)).toEqual(['0', 'region', 'Scrollable table']);
		expect(stop(narrow)).toEqual([null, null, null]);
		widths(wide, 900, 1200);
		resized();
		expect(stop(wide)).toEqual([null, null, null]);
	});

	it('checks the tables again when the Markdown is replaced', async () => {
		stubResize();
		const box = mountBox('no table yet');
		box.innerHTML = renderMarkdown(table);
		const wrap = box.querySelector('.table-scroll')!;
		widths(wrap, 900, 320);
		// The check runs when the children change; give the observer its turn.
		box.append(document.createElement('p'));
		await new Promise((r) => setTimeout(r));
		expect(stop(wrap)).toEqual(['0', 'region', 'Scrollable table']);
	});
});
