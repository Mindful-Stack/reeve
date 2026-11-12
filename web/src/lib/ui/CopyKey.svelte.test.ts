// @vitest-environment jsdom
// web/src/lib/ui/CopyKey.svelte.test.ts
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import CopyKey from './CopyKey.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	vi.unstubAllGlobals();
});
function clipboard(write: (t: string) => Promise<void>) {
	vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText: vi.fn(write) } });
	return navigator.clipboard.writeText as ReturnType<typeof vi.fn>;
}

it('copies exactly the key, says so, and keeps the click and the drag from the card', async () => {
	const write = clipboard(() => Promise.resolve());
	const parent = document.createElement('article');
	parent.draggable = true;
	const parentClick = vi.fn();
	parent.addEventListener('click', parentClick);
	document.body.append(parent);
	app = mount(CopyKey, { target: parent, props: { key: 'GRA-633' } });
	const b = parent.querySelector('button')!;
	expect(b.getAttribute('aria-label')).toBe('Copy GRA-633');
	expect(b.getAttribute('draggable')).toBe('false');
	const drag = new Event('dragstart', { bubbles: true, cancelable: true });
	b.dispatchEvent(drag);
	expect(drag.defaultPrevented).toBe(true);
	b.click();
	expect(parentClick).not.toHaveBeenCalled();
	expect(write).toHaveBeenCalledWith('GRA-633');
	await vi.waitFor(() => {
		flushSync();
		expect(parent.querySelector('[role="status"]')!.textContent).toBe('Copied');
		expect(b.getAttribute('title')).toBe('Copied');
	});
});

it('says Copy failed when the clipboard refuses', async () => {
	clipboard(() => Promise.reject(new Error('denied')));
	app = mount(CopyKey, { target: document.body, props: { key: 'GRA-1' } });
	document.body.querySelector('button')!.click();
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('[role="status"]')!.textContent).toBe('Copy failed');
	});
});

it('says Copy failed when there is no clipboard at all (plain http)', async () => {
	vi.stubGlobal('navigator', { ...navigator, clipboard: undefined });
	app = mount(CopyKey, { target: document.body, props: { key: 'GRA-1' } });
	document.body.querySelector('button')!.click();
	await vi.waitFor(() => {
		flushSync();
		expect(document.body.querySelector('[role="status"]')!.textContent).toBe('Copy failed');
	});
});
