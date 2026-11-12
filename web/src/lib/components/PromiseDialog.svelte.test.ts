// @vitest-environment jsdom
// The promise dialog: shortcuts only fill the date, the body names only what changed, and a recipient-only change on an
// overdue promise sends no date. The parent owns the write.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import type { PromiseFact, SetPromise } from '#lib/api/types.ts';
import PromiseDialog from './PromiseDialog.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

type Props = {
	activeEnds?: string | null;
	draftEnds?: string | null;
	today?: string | null;
	current?: PromiseFact | null;
	epicNote?: string | null;
	busy?: boolean;
	error?: string | null;
};

function open(over: Props = {}) {
	const saved: SetPromise[] = [];
	let closed = 0;
	app = mount(PromiseDialog, {
		target: document.body,
		props: {
			key: 'ABC-12',
			activeEnds: '2026-11-24',
			draftEnds: '2026-12-08',
			today: '2026-11-12',
			current: null,
			epicNote: null,
			busy: false,
			error: null,
			onsave: (b: SetPromise) => saved.push(b),
			onclose: () => closed++,
			...over
		}
	});
	flushSync();
	return { saved, closed: () => closed };
}

const q = <T extends Element>(s: string) => document.body.querySelector<T>(s);
const buttons = () => [...document.body.querySelectorAll<HTMLButtonElement>('button')];
const button = (label: string) => buttons().find((b) => b.textContent?.trim() === label);
const dateInput = () => q<HTMLInputElement>('input[type="date"]')!;
const toInput = () => q<HTMLInputElement>('input[name="to"]')!;
function type(el: HTMLInputElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
}
function submit() {
	button('Promise')!.click();
	flushSync();
}

const fact = (on: string, to: string | null = 'Acme'): PromiseFact => ({ on, to, by: 'dana@example.com', at: '2026-11-01T09:00:00Z' });

describe('PromiseDialog', () => {
	it('titles a new promise and a change', () => {
		open();
		expect(q('dialog')?.textContent).toContain('Promise ABC-12 by…');
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		open({ current: fact('2026-11-30') });
		expect(q('dialog')?.textContent).toContain('Change the promise on ABC-12');
	});

	it('shows both shortcuts with their dates, and min is today', () => {
		open();
		expect(button('End of this sprint (24 Nov)')).toBeDefined();
		expect(button('End of next sprint (8 Dec)')).toBeDefined();
		expect(dateInput().min).toBe('2026-11-12');
		expect(q('label')?.textContent).toBeDefined();
		expect(document.body.textContent).toContain('To whom (optional)');
	});

	it('hides a shortcut whose end is null or past, and both without today', () => {
		open({ activeEnds: '2026-11-11', draftEnds: null });
		expect(buttons().some((b) => b.textContent?.includes('End of'))).toBe(false);
		unmount(app!);
		app = null;
		open({ today: null });
		expect(buttons().some((b) => b.textContent?.includes('End of'))).toBe(false);
		expect(dateInput().hasAttribute('min')).toBe(false);
	});

	it('shows a shortcut ending today', () => {
		open({ activeEnds: '2026-11-12' });
		expect(button('End of this sprint (12 Nov)')).toBeDefined();
	});

	it('a shortcut fills the date without saving; submitting then sends the shortcut', () => {
		const { saved } = open();
		button('End of next sprint (8 Dec)')!.click();
		flushSync();
		expect(dateInput().value).toBe('2026-12-08');
		expect(saved).toEqual([]);
		submit();
		expect(saved).toEqual([{ shortcut: 'next_sprint' }]);
	});

	it('a shortcut then an edited date sends the date', () => {
		const { saved } = open();
		button('End of this sprint (24 Nov)')!.click();
		flushSync();
		type(dateInput(), '2026-11-27');
		submit();
		expect(saved).toEqual([{ on: '2026-11-27' }]);
	});

	it('an edited date sends {on}, with a typed recipient', () => {
		const { saved } = open();
		type(dateInput(), '2026-11-20');
		type(toInput(), ' Acme ');
		submit();
		expect(saved).toEqual([{ on: '2026-11-20', to: 'Acme' }]);
	});

	it('cannot submit a new promise without a date', () => {
		open();
		expect(button('Promise')!.disabled).toBe(true);
	});

	it('prefills the recipient; untouched it is not sent, emptied it is sent as ""', () => {
		const { saved } = open({ current: fact('2026-11-30') });
		expect(toInput().value).toBe('Acme');
		expect(dateInput().value).toBe('2026-11-30');
		type(dateInput(), '2026-11-28');
		submit();
		expect(saved).toEqual([{ on: '2026-11-28' }]);
		type(toInput(), '');
		submit();
		expect(saved[1]).toEqual({ on: '2026-11-28', to: '' });
	});

	it('a recipient-only change on an overdue promise sends {to} only, and min keeps the current date', () => {
		const { saved } = open({ current: fact('2026-11-05') });
		expect(dateInput().min).toBe('2026-11-05');
		type(toInput(), 'Beta');
		submit();
		expect(saved).toEqual([{ to: 'Beta' }]);
	});

	it('shows the epic note', () => {
		open({ epicNote: 'Epic ABC-7 is promised 18 Nov; the earlier date counts.' });
		expect(document.body.textContent).toContain('Epic ABC-7 is promised 18 Nov; the earlier date counts.');
	});

	it('busy disables the controls and error renders as an alert', () => {
		open({ busy: true, error: 'No.' });
		for (const b of buttons().filter((b) => !b.classList.contains('close'))) expect(b.disabled, b.textContent ?? '').toBe(true);
		expect(dateInput().disabled).toBe(true);
		expect(toInput().disabled).toBe(true);
		expect(q('[role="alert"]')?.textContent).toBe('No.');
	});

	it('Cancel reports onclose', () => {
		const d = open();
		button('Cancel')!.click();
		expect(d.closed()).toBe(1);
	});
});
