// @vitest-environment jsdom
// The promise dialog: shortcuts only fill the date, the body names only what changed, and a recipient-only change on an
// overdue promise sends no date. The parent owns the write.
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import type { PromiseFact, SetPromise, Sprints } from '#lib/api/types.ts';
import { sprint, sprintView } from '#lib/test/fixtures.ts';
import PromiseDialog from './PromiseDialog.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

/** The sprints the host holds: the active sprint ends on `activeEnds`, the draft on `draftEnds`, the server's date is `today`. */
function sprintsOf(activeEnds: string | null, draftEnds: string | null, today: string | null): Sprints | null {
	if (today == null && activeEnds == null && draftEnds == null) return null;
	return {
		active: activeEnds ? sprintView(sprint({ ends_on: activeEnds })) : null,
		draft: draftEnds ? sprint({ id: 2, state: 'draft', ends_on: draftEnds }) : null,
		closed: [],
		today
	};
}

type Props = {
	activeEnds?: string | null;
	draftEnds?: string | null;
	today?: string | null;
	sprintsFailed?: boolean;
	current?: PromiseFact | null;
	epic?: { key: string; on: string } | null;
	busy?: boolean;
	error?: string | null;
};

function open({ activeEnds = '2026-11-24', draftEnds = '2026-12-08', today = '2026-11-12', ...over }: Props = {}) {
	const saved: SetPromise[] = [];
	let closed = 0;
	app = mount(PromiseDialog, {
		target: document.body,
		props: {
			key: 'ABC-12',
			sprints: sprintsOf(activeEnds, draftEnds, today),
			current: null,
			epic: null,
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
const submitButton = () => q<HTMLButtonElement>('button[type="submit"]')!;
function submit() {
	submitButton().click();
	flushSync();
}

const fact = (on: string, to: string | null = 'Acme'): PromiseFact => ({ on, to, by: 'dana@example.com', at: '2026-11-01T09:00:00Z' });

describe('PromiseDialog', () => {
	it('titles a new promise and a change, and its submit says Promise or Save', () => {
		open();
		expect(q('dialog')?.textContent).toContain('Promise ABC-12 by…');
		expect(submitButton().textContent).toBe('Promise');
		unmount(app!);
		app = null;
		document.body.innerHTML = '';
		open({ current: fact('2026-11-30') });
		expect(q('dialog')?.textContent).toContain('Change the promise on ABC-12');
		expect(submitButton().textContent).toBe('Save');
	});

	it('shows both shortcuts with their dates, and min is today', () => {
		open();
		expect(button('End of this sprint (24 Nov)')).toBeDefined();
		expect(button('End of next sprint (8 Dec)')).toBeDefined();
		expect(dateInput().min).toBe('2026-11-12');
		expect(dateInput().closest('label')?.textContent?.trim()).toBe('Date');
		expect(toInput().closest('label')?.textContent?.trim()).toBe('To whom (optional)');
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

	it('reserves the shortcut row until the sprints are known, and collapses it when none can be offered', () => {
		open({ today: null });
		expect(q('[data-testid="promise-shortcuts"]')).not.toBeNull();
		unmount(app!);
		app = null;
		open({ activeEnds: '2026-11-11', draftEnds: null });
		expect(q('[data-testid="promise-shortcuts"]')).toBeNull();
		unmount(app!);
		app = null;
		open();
		expect(q('[data-testid="promise-shortcuts"]')?.querySelectorAll('button')).toHaveLength(2);
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

	it('the epic note follows the chosen date', () => {
		open({ epic: { key: 'ABC-7', on: '2026-11-18' } });
		const note = () => q('.note')?.textContent;
		expect(note()).toBe('Epic ABC-7 is promised by 18 Nov 2026.');
		type(dateInput(), '2026-11-30');
		expect(note()).toBe('Epic ABC-7 is promised by 18 Nov 2026; the earlier date counts.');
		type(dateInput(), '2026-11-18');
		expect(note()).toBe('Epic ABC-7 is promised by 18 Nov 2026.');
		type(dateInput(), '2026-11-14');
		expect(note()).toBe('Epic ABC-7 is promised by 18 Nov 2026.');
	});

	it('the epic note starts from the current date on a change', () => {
		open({ current: fact('2026-11-30'), epic: { key: 'ABC-7', on: '2026-11-18' } });
		expect(q('.note')?.textContent).toBe('Epic ABC-7 is promised by 18 Nov 2026; the earlier date counts.');
	});

	it('shows no note without an inherited epic promise', () => {
		open();
		expect(q('.note')).toBeNull();
		expect(dateInput().hasAttribute('aria-describedby')).toBe(false);
	});

	it('the epic note describes the date and is announced as it changes', () => {
		open({ epic: { key: 'ABC-7', on: '2026-11-18' } });
		const note = q<HTMLElement>('.note')!;
		expect(note.id).not.toBe('');
		expect(dateInput().getAttribute('aria-describedby')).toBe(note.id);
		expect(note.getAttribute('aria-live')).toBe('polite');
	});

	it('collapses the shortcut row and says so when the sprints could not be read', () => {
		open({ today: null, activeEnds: null, draftEnds: null, sprintsFailed: true });
		expect(q('[data-testid="promise-shortcuts"]')).toBeNull();
		expect(q('[data-testid="promise-shortcuts-unavailable"]')?.textContent).toBe('Sprint shortcuts are unavailable.');
		unmount(app!);
		app = null;
		// Still loading: the row is reserved and nothing is said.
		open({ today: null, activeEnds: null, draftEnds: null });
		expect(q('[data-testid="promise-shortcuts"]')).not.toBeNull();
		expect(q('[data-testid="promise-shortcuts-unavailable"]')).toBeNull();
	});

	it("sends the date in the field when the shortcut's end moved after the pick", () => {
		const props = $state({ activeEnds: '2026-11-24' as string | null });
		const saved: SetPromise[] = [];
		app = mount(PromiseDialog, {
			target: document.body,
			props: {
				key: 'ABC-12',
				get sprints() {
					return sprintsOf(props.activeEnds, null, '2026-11-12');
				},
				onsave: (b: SetPromise) => saved.push(b),
				onclose: () => {}
			}
		});
		flushSync();
		button('End of this sprint (24 Nov)')!.click();
		flushSync();
		// A live refetch moves the sprint's end under the open dialog: the field still says 24 Nov.
		props.activeEnds = '2026-11-26';
		flushSync();
		expect(dateInput().value).toBe('2026-11-24');
		expect(button('End of this sprint (26 Nov)')!.getAttribute('aria-pressed')).toBe('false');
		submit();
		expect(saved).toEqual([{ on: '2026-11-24' }]);
		// The sprint closes: no shortcut is offered any more, and the date is still what is sent.
		props.activeEnds = null;
		flushSync();
		submit();
		expect(saved[1]).toEqual({ on: '2026-11-24' });
	});

	it('counts the recipient in code points, as the server does: 200 supplementary characters pass, 201 do not', () => {
		const { saved } = open();
		type(dateInput(), '2026-11-20');
		const emoji = '\u{1F600}';
		type(toInput(), emoji.repeat(201));
		expect(toInput().value).toHaveLength(402);
		expect(toInput().validationMessage).toBe('the recipient is longer than 200 characters');
		submit();
		expect(saved).toEqual([]);
		type(toInput(), emoji.repeat(200));
		expect(toInput().validationMessage).toBe('');
		submit();
		expect(saved).toEqual([{ on: '2026-11-20', to: emoji.repeat(200) }]);
		// Surrounding spaces are not counted: the server trims first.
		type(toInput(), ` ${emoji.repeat(200)} `);
		expect(toInput().validationMessage).toBe('');
	});

	it('a shortcut with a typed recipient sends {shortcut, to}', () => {
		const { saved } = open();
		button('End of this sprint (24 Nov)')!.click();
		flushSync();
		type(toInput(), 'Acme');
		submit();
		expect(saved).toEqual([{ shortcut: 'this_sprint', to: 'Acme' }]);
	});

	it('an untouched change cannot be submitted; any edit enables it', () => {
		open({ current: fact('2026-11-30') });
		expect(button('Save')!.disabled).toBe(true);
		type(toInput(), 'Beta');
		expect(button('Save')!.disabled).toBe(false);
		type(toInput(), 'Acme');
		expect(button('Save')!.disabled).toBe(true);
		type(dateInput(), '2026-11-28');
		expect(button('Save')!.disabled).toBe(false);
	});

	it('keeps its title when the current promise changes under it', () => {
		const props = $state({ current: null as PromiseFact | null });
		app = mount(PromiseDialog, {
			target: document.body,
			props: {
				key: 'ABC-12',
				today: '2026-11-12',
				get current() {
					return props.current;
				},
				onsave: () => {},
				onclose: () => {}
			}
		});
		flushSync();
		expect(q('dialog h2')?.textContent).toBe('Promise ABC-12 by…');
		props.current = fact('2026-11-30');
		flushSync();
		expect(q('dialog h2')?.textContent).toBe('Promise ABC-12 by…');
	});

	it('on an overdue promise, a changed date before today is invalid and not sent', () => {
		const { saved } = open({ current: fact('2026-11-05') });
		expect(dateInput().validationMessage).toBe('');
		type(dateInput(), '2026-11-08');
		expect(dateInput().validationMessage).toBe('the date is past: promise today or later');
		submit();
		expect(saved).toEqual([]);
		type(dateInput(), '2026-11-05');
		expect(dateInput().validationMessage).toBe('');
		type(dateInput(), '2026-11-12');
		expect(dateInput().validationMessage).toBe('');
		submit();
		expect(saved).toEqual([{ on: '2026-11-12' }]);
	});

	it('focuses the date again when a save fails', async () => {
		const props = $state({ busy: false, error: null as string | null });
		app = mount(PromiseDialog, {
			target: document.body,
			props: {
				key: 'ABC-12',
				today: '2026-11-12',
				get busy() {
					return props.busy;
				},
				get error() {
					return props.error;
				},
				onsave: () => {},
				onclose: () => {}
			}
		});
		flushSync();
		props.busy = true;
		flushSync();
		// A browser drops focus from a control that becomes disabled; jsdom keeps it, so move it by hand.
		const elsewhere = document.createElement('button');
		document.body.append(elsewhere);
		elsewhere.focus();
		expect(document.activeElement).toBe(elsewhere);
		props.busy = false;
		props.error = 'boom';
		flushSync();
		await tick();
		await tick();
		expect(document.activeElement).toBe(dateInput());
		expect(q('[role="alert"]')?.textContent).toBe('boom');
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
