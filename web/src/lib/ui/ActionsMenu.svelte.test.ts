// @vitest-environment jsdom
// web/src/lib/ui/ActionsMenu.svelte.test.ts
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ActionsMenu from './ActionsMenu.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});
const key = (el: Element, k: string) => el.dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));

async function open(items: { label: string; onselect: () => void; disabled?: boolean }[]) {
	app = mount(ActionsMenu, { target: document.body, props: { label: 'Actions for ABC-1', items, control: 'menu' } });
	flushSync(); // bind:this is assigned in an effect, which mount() does not flush
	const trigger = document.body.querySelector('button[aria-haspopup="menu"]') as HTMLButtonElement;
	trigger.click();
	flushSync();
	await tick();
	return trigger;
}

describe('ActionsMenu under the top bar', () => {
	afterEach(() => document.documentElement.style.removeProperty('--top-bar-height'));
	const rect = (top: number, bottom: number) => ({ top, bottom, left: 10, right: 60, width: 50, height: bottom - top, x: 10, y: top, toJSON() {} }) as DOMRect;

	it('closes when its trigger scrolls behind the bar', async () => {
		document.documentElement.style.setProperty('--top-bar-height', '60px');
		let at = rect(100, 120); // below the bar (bottom 60)
		const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
			return this.matches('button[aria-haspopup="menu"]') ? at : rect(0, 0);
		});
		const trigger = await open([{ label: 'Move up', onselect: vi.fn() }]);
		expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
		// Trigger bottom (50) is above the bar's bottom (60), but still inside the viewport.
		at = rect(20, 50);
		window.dispatchEvent(new Event('scroll'));
		flushSync();
		await tick();
		expect(document.body.querySelector('[role="menu"]')).toBeNull();
		expect(trigger.getAttribute('aria-expanded')).toBe('false');
		spy.mockRestore();
	});

	it('stays open for a trigger inside the top bar, whose bottom is always above the bar bottom', async () => {
		document.documentElement.style.setProperty('--top-bar-height', '60px');
		const header = document.createElement('header');
		header.className = 'top-bar';
		document.body.append(header);
		const at = rect(10, 40); // inside the bar
		const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
			return this.matches('button[aria-haspopup="menu"]') ? at : rect(0, 0);
		});
		app = mount(ActionsMenu, { target: header, props: { label: 'Account', items: [{ label: 'Sign out', onselect: vi.fn() }], control: 'menu' } });
		flushSync();
		(header.querySelector('button[aria-haspopup="menu"]') as HTMLButtonElement).click();
		flushSync();
		await tick();
		window.dispatchEvent(new Event('scroll'));
		flushSync();
		await tick();
		expect(document.body.querySelector('[role="menu"]')).not.toBeNull();
		spy.mockRestore();
	});

	it('keeps a page menu below the bar, within the viewport, and off its own trigger', async () => {
		document.documentElement.style.setProperty('--top-bar-height', '60px');
		const innerHeight = window.innerHeight; // jsdom: 768
		// A trigger near the bottom with a tall menu: it flips up, and there is only room down to the bar.
		let at = rect(innerHeight - 40, innerHeight - 10);
		const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
			if (this.matches('button[aria-haspopup="menu"]')) return at;
			if (this.matches('[role="menu"]')) return rect(0, 900); // taller than the viewport
			return rect(0, 0);
		});
		const trigger = await open([{ label: 'Move up', onselect: vi.fn() }]);
		const menu = document.body.querySelector('[role="menu"]') as HTMLElement;
		const top = parseFloat(menu.style.top);
		const maxH = parseFloat(menu.style.maxHeight);
		expect(top).toBeGreaterThanOrEqual(60 + 8);
		expect(top + maxH).toBeLessThanOrEqual(at.top - 4 + 0.5); // ends above the trigger
		expect(top + maxH).toBeLessThanOrEqual(innerHeight - 8);
		expect(trigger.isConnected).toBe(true);
		spy.mockRestore();
	});
});

describe('ActionsMenu', () => {
	it('opens a menu, focuses the first enabled item and moves with arrows', async () => {
		const trigger = await open([
			{ label: 'Move to in progress', onselect: vi.fn() },
			{ label: 'Move up', onselect: vi.fn(), disabled: true },
			{ label: 'Move down', onselect: vi.fn() }
		]);
		expect(trigger.getAttribute('aria-expanded')).toBe('true');
		const items = [...document.body.querySelectorAll('[role="menuitem"]')] as HTMLButtonElement[];
		expect(document.activeElement).toBe(items[0]);
		key(document.activeElement!, 'ArrowDown');
		expect(document.activeElement).toBe(items[2]); // skips the disabled item
		key(document.activeElement!, 'ArrowDown');
		expect(document.activeElement).toBe(items[0]); // wraps
	});
	it('Escape closes and returns focus to the trigger', async () => {
		const trigger = await open([{ label: 'Move up', onselect: vi.fn() }]);
		key(document.activeElement!, 'Escape');
		flushSync();
		expect(document.body.querySelector('[role="menu"]')).toBeNull();
		expect(document.activeElement).toBe(trigger);
	});
	it('choosing an item closes the menu, refocuses the trigger, then runs the action', async () => {
		const order: string[] = [];
		const trigger = await open([{ label: 'Move up', onselect: () => order.push(`run:${document.activeElement === trigger}`) }]);
		(document.body.querySelector('[role="menuitem"]') as HTMLButtonElement).click();
		flushSync();
		expect(order).toEqual(['run:true']);
		expect(document.body.querySelector('[role="menu"]')).toBeNull();
	});
	it('keeps the same trigger with data-control when closed', async () => {
		const trigger = await open([{ label: 'Move up', onselect: vi.fn() }]);
		key(document.activeElement!, 'Escape');
		flushSync();
		expect(trigger.isConnected).toBe(true);
		expect(trigger.dataset.control).toBe('menu');
		expect(trigger.getAttribute('aria-expanded')).toBe('false');
	});
	it('ArrowUp, Home and End move focus; Tab closes without refocusing', async () => {
		await open([
			{ label: 'A', onselect: vi.fn() },
			{ label: 'B', onselect: vi.fn() },
			{ label: 'C', onselect: vi.fn() }
		]);
		const items = [...document.body.querySelectorAll('[role="menuitem"]')] as HTMLButtonElement[];
		key(document.activeElement!, 'ArrowUp');
		expect(document.activeElement).toBe(items[2]);
		key(document.activeElement!, 'Home');
		expect(document.activeElement).toBe(items[0]);
		key(document.activeElement!, 'End');
		expect(document.activeElement).toBe(items[2]);
		key(document.activeElement!, 'Tab');
		flushSync();
		expect(document.body.querySelector('[role="menu"]')).toBeNull();
		expect(document.activeElement).not.toBe(document.body.querySelector('button[aria-haspopup]'));
	});
	const down = (el: Element) => {
		const ev = new MouseEvent('mousedown', { bubbles: true, cancelable: true });
		el.dispatchEvent(ev);
		flushSync();
		return ev;
	};
	it('outside mousedown on plain ground is prevented and refocuses the trigger', async () => {
		const trigger = await open([{ label: 'A', onselect: vi.fn() }]);
		expect(down(document.body).defaultPrevented).toBe(true);
		expect(document.body.querySelector('[role="menu"]')).toBeNull();
		expect(document.activeElement).toBe(trigger);
	});
	it('outside mousedown on a disabled button or an anchor without href is plain ground', async () => {
		const dis = document.createElement('button');
		dis.disabled = true;
		const a = document.createElement('a');
		document.body.append(dis, a);
		const trigger = await open([{ label: 'A', onselect: vi.fn() }]);
		expect(down(dis).defaultPrevented).toBe(true);
		expect(document.activeElement).toBe(trigger);
		trigger.click();
		flushSync();
		await tick();
		expect(down(a).defaultPrevented).toBe(true);
		expect(document.activeElement).toBe(trigger);
	});
	it('outside mousedown on an enabled button is not prevented and does not refocus the trigger', async () => {
		const other = document.createElement('button');
		document.body.append(other);
		const trigger = await open([{ label: 'A', onselect: vi.fn() }]);
		expect(down(other).defaultPrevented).toBe(false);
		expect(document.body.querySelector('[role="menu"]')).toBeNull();
		expect(document.activeElement).not.toBe(trigger);
	});
	it.each([
		['above', { top: -80, bottom: -50 }],
		['sideways', { left: -100, right: -72 }]
	])('closes and refocuses the trigger when it scrolls out of the viewport %s', async (_n, off) => {
		let rect = { top: 100, bottom: 130, left: 20, right: 48, width: 28, height: 30 } as DOMRect;
		const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => rect);
		try {
			const trigger = await open([{ label: 'A', onselect: vi.fn() }]);
			const focus = vi.spyOn(trigger, 'focus');
			rect = { ...rect, ...off } as DOMRect;
			window.dispatchEvent(new Event('scroll'));
			flushSync();
			expect(document.body.querySelector('[role="menu"]')).toBeNull();
			expect(document.activeElement).toBe(trigger);
			expect(focus).toHaveBeenCalledWith({ preventScroll: true });
		} finally {
			spy.mockRestore();
		}
	});
	it('repositions on resize and on a nested scroll', async () => {
		let rect = { top: 100, bottom: 130, left: 20, right: 48, width: 28, height: 30 } as DOMRect;
		const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(() => rect);
		try {
			await open([{ label: 'A', onselect: vi.fn() }]);
			const menu = document.body.querySelector('[role="menu"]') as HTMLElement;
			expect(menu.style.top).toBe('134px');
			rect = { ...rect, top: 200, bottom: 230 } as DOMRect;
			window.dispatchEvent(new Event('resize'));
			flushSync();
			expect(menu.style.top).toBe('234px');
			rect = { ...rect, top: 300, bottom: 330 } as DOMRect;
			const child = document.createElement('div');
			document.body.append(child);
			child.dispatchEvent(new Event('scroll')); // does not bubble; reaches window only by capture
			flushSync();
			expect(menu.style.top).toBe('334px');
		} finally {
			spy.mockRestore();
		}
	});
	it('removes the listeners it added on close and on unmount', async () => {
		const wAdd = vi.spyOn(window, 'addEventListener');
		const wRem = vi.spyOn(window, 'removeEventListener');
		const dAdd = vi.spyOn(document, 'addEventListener');
		const dRem = vi.spyOn(document, 'removeEventListener');
		try {
			await open([{ label: 'A', onselect: vi.fn() }]);
			const added = (spy: typeof wAdd, type: string) => spy.mock.calls.find((c) => c[0] === type);
			const scroll = added(wAdd, 'scroll')!;
			const resize = added(wAdd, 'resize')!;
			const mouse = added(dAdd, 'mousedown')!;
			expect(scroll[2]).toMatchObject({ capture: true });
			key(document.activeElement!, 'Escape');
			flushSync();
			expect(wRem).toHaveBeenCalledWith('scroll', scroll[1], { capture: true });
			expect(wRem).toHaveBeenCalledWith('resize', resize[1]);
			expect(dRem).toHaveBeenCalledWith('mousedown', mouse[1]);
			// open again, then unmount while open
			wRem.mockClear();
			dRem.mockClear();
			(document.body.querySelector('button[aria-haspopup]') as HTMLButtonElement).click();
			flushSync();
			await tick();
			const scroll2 = wAdd.mock.calls.filter((c) => c[0] === 'scroll').at(-1)!;
			const mouse2 = dAdd.mock.calls.filter((c) => c[0] === 'mousedown').at(-1)!;
			unmount(app!);
			app = null;
			expect(wRem).toHaveBeenCalledWith('scroll', scroll2[1], { capture: true });
			expect(dRem).toHaveBeenCalledWith('mousedown', mouse2[1]);
		} finally {
			wAdd.mockRestore();
			wRem.mockRestore();
			dAdd.mockRestore();
			dRem.mockRestore();
		}
	});
	it('renders duplicate labels and runs each action', async () => {
		const a = vi.fn();
		const b = vi.fn();
		await open([
			{ label: 'Same', onselect: a },
			{ label: 'Same', onselect: b }
		]);
		const items = [...document.body.querySelectorAll('[role="menuitem"]')] as HTMLButtonElement[];
		expect(items).toHaveLength(2);
		items[1].click();
		flushSync();
		expect(a).not.toHaveBeenCalled();
		expect(b).toHaveBeenCalledOnce();
	});
	it('focuses the menu when no item is enabled, so Escape still closes', async () => {
		await open([{ label: 'A', onselect: vi.fn(), disabled: true }]);
		expect(document.activeElement).toBe(document.body.querySelector('[role="menu"]'));
		key(document.activeElement!, 'Escape');
		flushSync();
		expect(document.body.querySelector('[role="menu"]')).toBeNull();
	});
	it('flips above near the viewport bottom and repositions on scroll', async () => {
		const innerHeight = vi.spyOn(window, 'innerHeight', 'get').mockReturnValue(600);
		let rect = { top: 560, bottom: 590, left: 20, right: 48, width: 28, height: 30, x: 20, y: 560 } as DOMRect;
		const spy = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
			return this.getAttribute('role') === 'menu' ? ({ width: 200, height: 100, top: 0, left: 0, bottom: 100, right: 200 } as DOMRect) : rect;
		});
		try {
			await open([{ label: 'A', onselect: vi.fn() }]);
			const menu = document.body.querySelector('[role="menu"]') as HTMLElement;
			expect(menu.style.top).toBe('456px'); // 560 - 4 - 100
			rect = { ...rect, top: 100, bottom: 130 } as DOMRect;
			window.dispatchEvent(new Event('scroll'));
			flushSync();
			expect(menu.style.top).toBe('134px');
		} finally {
			spy.mockRestore();
			innerHeight.mockRestore();
		}
	});
});
