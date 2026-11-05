// @vitest-environment jsdom
// web/src/lib/ui/Toggle.svelte.test.ts
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Toggle from './Toggle.svelte';
import Harness from './ToggleHarness.test.svelte';

let apps: ReturnType<typeof mount>[] = [];
afterEach(() => {
	apps.forEach((a) => unmount(a));
	apps = [];
	document.body.innerHTML = '';
});
const options = [
	{ value: 'a', label: 'A' },
	{ value: 'b', label: 'B' },
	{ value: 'c', label: 'C' }
];

async function withUnhandled(run: () => Promise<void>) {
	const saved = process.listeners('unhandledRejection');
	process.removeAllListeners('unhandledRejection');
	const seen: unknown[] = [];
	const h = (e: unknown) => void seen.push(e);
	process.on('unhandledRejection', h);
	try {
		await run();
	} finally {
		process.off('unhandledRejection', h);
		saved.forEach((l) => process.on('unhandledRejection', l));
	}
	return seen;
}
type Deferred = { v: string; ok: () => void; fail: (e: Error) => void };
function tabs(extra: Record<string, unknown> = {}, accept = true, delayed = false, initial = 'a', deferred?: Deferred[]) {
	const s = $state({ value: initial });
	const onchange = vi.fn((v: string) => {
		if (deferred)
			return new Promise<void>((res, rej) => {
				deferred.push({ v, ok: () => ((s.value = v), res()), fail: rej });
			});
		if (!accept) return;
		if (delayed) return new Promise((r) => setTimeout(r, 0)).then(() => void (s.value = v));
		s.value = v;
	});
	apps.push(
		mount(Toggle, {
			target: document.body,
			props: {
				label: 'T',
				mode: 'tabs',
				options,
				get value() {
					return s.value;
				},
				onchange,
				...extra
			}
		})
	);
	flushSync();
	const els = [...document.body.querySelectorAll('[role="tab"]')] as HTMLButtonElement[];
	els[0].focus();
	const press = (key: string) => {
		(document.activeElement as HTMLElement).dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
		flushSync();
	};
	const expectAt = (i: number) => {
		els.forEach((el, j) => {
			expect(el.getAttribute('aria-selected')).toBe(String(i === j));
			expect(el.tabIndex).toBe(i === j ? 0 : -1);
		});
		expect(document.activeElement).toBe(els[i]);
	};
	return { els, press, expectAt, onchange, s };
}

describe('Toggle', () => {
	it('segmented: one pressed button per option, clicking reports the value', () => {
		const onchange = vi.fn();
		apps.push(mount(Toggle, { target: document.body, props: { label: 'Who', options, value: 'a', onchange } }));
		const [a, b] = document.body.querySelectorAll('button');
		expect(a.getAttribute('aria-pressed')).toBe('true');
		expect(b.getAttribute('aria-pressed')).toBe('false');
		b.click();
		flushSync();
		expect(onchange).toHaveBeenCalledWith('b');
	});
	it('clicking the selected option does not call onchange', () => {
		const onchange = vi.fn();
		apps.push(mount(Toggle, { target: document.body, props: { label: 'Who', options, value: 'a', onchange } }));
		document.body.querySelector('button')!.click();
		expect(onchange).not.toHaveBeenCalled();
	});
	it('segmented ignores arrow keys', () => {
		const onchange = vi.fn();
		apps.push(mount(Toggle, { target: document.body, props: { label: 'Who', options, value: 'a', onchange } }));
		document.body.querySelector('button')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true }));
		expect(onchange).not.toHaveBeenCalled();
	});
	it('tabs: arrows, wrap, Home and End move selection and focus', () => {
		const { press, expectAt } = tabs();
		expectAt(0);
		press('ArrowRight');
		expectAt(1);
		press('ArrowRight');
		expectAt(2);
		press('ArrowRight');
		expectAt(0);
		press('ArrowLeft');
		expectAt(2);
		press('ArrowLeft');
		expectAt(1);
		press('Home');
		expectAt(0);
		press('End');
		expectAt(2);
	});
	it('tabs: a rejecting parent leaves focus and selection in place', () => {
		const { press, expectAt, onchange } = tabs({}, false);
		press('ArrowRight');
		expect(onchange).toHaveBeenCalledWith('b');
		expectAt(0);
	});
	it('tabs: a rejected change does not leave focus following a later external change', async () => {
		const { press, els, s } = tabs({}, false);
		press('ArrowRight');
		await new Promise((r) => setTimeout(r, 20));
		s.value = 'c';
		flushSync();
		expect(els[2].getAttribute('aria-selected')).toBe('true');
		expect(document.activeElement).toBe(els[0]);
	});
	it('tabs: a delayed acceptance still moves focus to the new tab', async () => {
		const { press, els } = tabs({}, true, true);
		press('ArrowRight');
		await new Promise((r) => setTimeout(r, 20));
		flushSync();
		expect(els[1].getAttribute('aria-selected')).toBe('true');
		expect(document.activeElement).toBe(els[1]);
	});
	it('tabs: an unknown value leaves tab 0 in the tab order', () => {
		const { els } = tabs({}, true, false, 'zzz');
		expect(els.map((e) => e.tabIndex)).toEqual([0, -1, -1]);
	});
	it('tabs: a tabpanel is labelled by the selected tab and aria-controls follows the selection', () => {
		apps.push(mount(Harness, { target: document.body, props: { options } }));
		flushSync();
		const els = [...document.body.querySelectorAll('[role="tab"]')] as HTMLButtonElement[];
		const panel = document.body.querySelector('[role="tabpanel"]')!;
		expect(panel.getAttribute('aria-labelledby')).toBe(els[0].id);
		expect(els.map((e) => e.getAttribute('aria-controls'))).toEqual(['pan', null, null]);
		els[2].click();
		flushSync();
		expect(panel.getAttribute('aria-labelledby')).toBe(els[2].id);
		expect(els.map((e) => e.getAttribute('aria-controls'))).toEqual([null, null, 'pan']);
	});
	const settle = () => new Promise((r) => setTimeout(r, 20));
	it('tabs: overlapping presses, first rejected then second accepted, focus lands on the End target', async () => {
		const d: Deferred[] = [];
		const { press, els } = tabs({}, true, false, 'a', d);
		press('ArrowRight');
		press('End');
		const seen = await withUnhandled(async () => {
			d[0].fail(new Error('no'));
			await settle();
			d[1].ok();
			flushSync();
			await settle();
		});
		expect(seen).toHaveLength(1);
		expect(document.activeElement).toBe(els[2]);
	});
	it('tabs: overlapping presses both accepted, focus ends on the End target', async () => {
		const d: Deferred[] = [];
		const { press, els } = tabs({}, true, false, 'a', d);
		press('ArrowRight');
		press('End');
		d[0].ok();
		flushSync();
		await settle();
		d[1].ok();
		flushSync();
		await settle();
		expect(document.activeElement).toBe(els[2]);
	});
	it('tabs: a rejected callback error is rethrown as unhandled once', async () => {
		const d: Deferred[] = [];
		const { press } = tabs({}, true, false, 'a', d);
		press('ArrowRight');
		const seen = await withUnhandled(async () => {
			d[0].fail(new Error('boom'));
			await settle();
		});
		expect(seen.map((e) => (e as Error).message)).toEqual(['boom']);
	});
	it('tabs: panel sets aria-controls on the selected tab only; ids are unique per instance', () => {
		const first = tabs({ panel: 'p1' });
		expect(first.els.map((e) => e.getAttribute('aria-controls'))).toEqual(['p1', null, null]);
		const second = tabs({ panel: 'p2' });
		const ids = [...first.els, ...second.els].map((e) => e.id);
		expect(new Set(ids).size).toBe(6);
	});
});
