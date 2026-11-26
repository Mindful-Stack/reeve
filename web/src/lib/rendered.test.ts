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
	// jsdom lays nothing out: a wrapper's widths are set by hand, and the stub hands back what a resize would call and
	// records what is watched.
	let resized: () => void = () => {};
	let observed: Element[] = [];
	let disconnects = 0;
	function stubResize(): void {
		observed = [];
		disconnects = 0;
		vi.stubGlobal(
			'ResizeObserver',
			class {
				constructor(cb: () => void) {
					resized = cb;
				}
				observe(target: Element) {
					observed.push(target);
				}
				unobserve(target: Element) {
					observed = observed.filter((o) => o !== target);
				}
				disconnect() {
					observed = [];
					disconnects += 1;
				}
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
		expect(stop(box.querySelector('[data-table-scroll]')!)).toEqual([null, null, null]);
	});

	it('makes a wrapper that scrolls sideways a named tab stop, and takes that back when the table fits again', () => {
		stubResize();
		const box = mountBox(`${table}\n\ntext\n\n${table}`);
		const [wide, narrow] = box.querySelectorAll('[data-table-scroll]');
		widths(wide, 900, 320);
		widths(narrow, 200, 320);
		resized();
		expect(stop(wide)).toEqual(['0', 'region', 'Table: a, b']);
		expect(stop(narrow)).toEqual([null, null, null]);
		widths(wide, 900, 1200);
		resized();
		expect(stop(wide)).toEqual([null, null, null]);
	});

	it('names the region after its table: the caption, else the first header cells, else just "Table"', () => {
		stubResize();
		const box = mountBox(
			[
				'<table><caption> Budget\n per year </caption><tr><th>Year</th><th>Sum</th></tr></table>',
				'| Step | User | Phone | Watch | System |\n|---|---|---|---|---|\n| 1 | 2 | 3 | 4 | 5 |',
				'|  Step  | | *Phone* |\n|---|---|---|\n| 1 | 2 | 3 |',
				'<table><tr><td>1</td><td>2</td></tr></table>'
			].join('\n\n')
		);
		const wraps = [...box.querySelectorAll('[data-table-scroll]')];
		for (const wrap of wraps) widths(wrap, 900, 320);
		resized();
		expect(wraps.map((w) => w.getAttribute('aria-label'))).toEqual([
			'Budget per year',
			'Table: Step, User, Phone…',
			'Table: Step, Phone',
			'Table'
		]);
	});

	it('keeps the name to 80 characters, an ellipsis last, however long the caption or the headers are', () => {
		stubResize();
		const exact = 'c'.repeat(80);
		const box = mountBox(
			[
				`<table><caption>${'Budget  per\n year '.repeat(30)}</caption><tr><th>Year</th></tr></table>`,
				`| ${'a'.repeat(60)} | ${'b'.repeat(60)} | ${'c'.repeat(60)} |\n|---|---|---|\n| 1 | 2 | 3 |`,
				`<table><caption>${exact}</caption><tr><td>1</td></tr></table>`,
				`<table><caption>${exact}d</caption><tr><td>1</td></tr></table>`
			].join('\n\n')
		);
		const wraps = [...box.querySelectorAll('[data-table-scroll]')];
		for (const wrap of wraps) widths(wrap, 900, 320);
		resized();
		const names = wraps.map((w) => w.getAttribute('aria-label')!);
		expect(names.map((n) => n.length)).toEqual([80, 80, 80, 80]);
		// Whitespace is one space before the cut, and the cut leaves no space before the ellipsis.
		expect(names[0]).toMatch(/^Budget per year Budget per year .*\S…$/);
		expect(names[0]).not.toMatch(/\s{2}/);
		expect(names[1]).toBe(`Table: ${'a'.repeat(60)}, ${'b'.repeat(10)}…`);
		// A name of exactly the limit is kept whole; one character more is cut.
		expect(names[2]).toBe(exact);
		expect(names[3]).toBe(`${'c'.repeat(79)}…`);
	});

	it('leaves a table alone whose author gave it the class of a wrapper', () => {
		stubResize();
		const box = mountBox('<table class="table-scroll" aria-label="Budget" tabindex="0"><tr><td>1</td></tr></table>');
		const authored = box.querySelector('table')!;
		expect([authored.getAttribute('aria-label'), authored.getAttribute('tabindex')]).toEqual(['Budget', '0']);
		// Only the wrapper `renderMarkdown` made is the action's to change.
		const wrap = authored.parentElement!;
		widths(wrap, 900, 320);
		resized();
		expect(wrap.getAttribute('tabindex')).toBe('0');
		expect([authored.getAttribute('aria-label'), authored.getAttribute('tabindex'), authored.getAttribute('role')]).toEqual(['Budget', '0', null]);
	});

	it('checks the tables again when the Markdown is replaced', async () => {
		stubResize();
		const box = mountBox('no table yet');
		box.innerHTML = renderMarkdown(table);
		const wrap = box.querySelector('[data-table-scroll]')!;
		widths(wrap, 900, 320);
		// The check runs when the children change; give the observer its turn.
		box.append(document.createElement('p'));
		await new Promise((r) => setTimeout(r));
		expect(stop(wrap)).toEqual(['0', 'region', 'Table: a, b']);
	});

	it('watches the container, each wrapper and each table, so a late image or font that widens a table is seen', () => {
		stubResize();
		const box = mountBox(`${table}\n\ntext\n\n${table}`);
		const wraps = [...box.querySelectorAll('[data-table-scroll]')];
		expect(new Set(observed)).toEqual(new Set([box, ...wraps, ...wraps.map((w) => w.firstElementChild!)]));
		expect(observed).toHaveLength(5);
	});

	it('stops watching the tables that went when the Markdown is replaced, and watches the new ones', async () => {
		stubResize();
		const box = mountBox(table);
		const old = box.querySelector('[data-table-scroll]')!;
		const oldTable = old.firstElementChild!;
		box.innerHTML = renderMarkdown(`${table}\n\ntext`);
		await new Promise((r) => setTimeout(r));
		const wrap = box.querySelector('[data-table-scroll]')!;
		expect(observed).not.toContain(old);
		expect(observed).not.toContain(oldTable);
		expect(new Set(observed)).toEqual(new Set([box, wrap, wrap.firstElementChild!]));
		expect(observed).toHaveLength(3);
		box.innerHTML = renderMarkdown('no table any more');
		await new Promise((r) => setTimeout(r));
		expect(observed).toEqual([box]);
	});

	it('lets go of everything when the container unmounts', () => {
		stubResize();
		const nodes = vi.spyOn(MutationObserver.prototype, 'disconnect');
		mountBox(table);
		expect(observed).toHaveLength(3);
		detach();
		detach = () => {};
		expect(observed).toEqual([]);
		expect(disconnects).toBe(1);
		expect(nodes).toHaveBeenCalledTimes(1);
		nodes.mockRestore();
	});
});
