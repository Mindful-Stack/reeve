// @vitest-environment jsdom
// web/src/lib/ui/CopyKey.svelte.test.ts
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import ActionsMenu from './ActionsMenu.svelte';
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
	// A press that is not cancelled lets the browser start the draggable card's drag (pinned for real in the smoke).
	const press = new Event('mousedown', { bubbles: true, cancelable: true });
	b.dispatchEvent(press);
	expect(press.defaultPrevented).toBe(true);
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

it('announces a second copy again and ignores a stale slower write', async () => {
	let first!: () => void;
	const calls: (() => Promise<void>)[] = [() => Promise.resolve(), () => Promise.resolve()];
	clipboard(() => calls.shift()!());
	app = mount(CopyKey, { target: document.body, props: { key: 'GRA-1' } });
	const status = () => document.body.querySelector('[role="status"]')!.textContent;
	const b = document.body.querySelector('button')!;
	b.click();
	await vi.waitFor(() => (flushSync(), expect(status()).toBe('Copied')));
	b.click();
	flushSync();
	expect(status()).toBe(''); // cleared at once, so the next "Copied" is a change a screen reader announces
	await vi.waitFor(() => (flushSync(), expect(status()).toBe('Copied')));

	// A slow failing write followed by a fast successful one: the stale failure must not overwrite it.
	calls.push(() => new Promise<void>((_, rej) => (first = () => rej(new Error('late')))), () => Promise.resolve());
	b.click();
	b.click();
	await vi.waitFor(() => (flushSync(), expect(status()).toBe('Copied')));
	first();
	await new Promise((r) => setTimeout(r, 10));
	flushSync();
	expect(status()).toBe('Copied');
});

it('shows a distinct icon and the title when the copy failed', async () => {
	clipboard(() => Promise.reject(new Error('denied')));
	app = mount(CopyKey, { target: document.body, props: { key: 'GRA-1' } });
	const b = document.body.querySelector('button')!;
	const before = b.innerHTML;
	b.click();
	await vi.waitFor(() => (flushSync(), expect(b.getAttribute('title')).toBe('Copy failed')));
	expect(b.innerHTML).not.toBe(before);
});

it('schedules nothing after unmount', async () => {
	vi.useFakeTimers();
	try {
		let done!: () => void;
		clipboard(() => new Promise<void>((r) => (done = r)));
		app = mount(CopyKey, { target: document.body, props: { key: 'GRA-1' } });
		flushSync(); // run the effect, which registers the cleanup
		document.body.querySelector('button')!.click();
		unmount(app);
		app = null;
		done();
		await Promise.resolve();
		await Promise.resolve();
		expect(vi.getTimerCount()).toBe(0);
	} finally {
		vi.useRealTimers();
	}
});

it('lets an open actions menu close when the copy button is pressed', async () => {
	app = mount(ActionsMenu, { target: document.body, props: { label: 'Actions for GRA-1', items: [{ label: 'Move up', onselect: () => {} }], control: 'menu' } });
	const copy = mount(CopyKey, { target: document.body, props: { key: 'GRA-1' } });
	flushSync();
	document.body.querySelector<HTMLButtonElement>('button[aria-haspopup="menu"]')!.click();
	flushSync();
	await vi.waitFor(() => expect(document.body.querySelector('[role="menu"]')).not.toBeNull());
	document.body.querySelector('button[aria-label="Copy GRA-1"]')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
	flushSync();
	expect(document.body.querySelector('[role="menu"]')).toBeNull();
	unmount(copy);
});
