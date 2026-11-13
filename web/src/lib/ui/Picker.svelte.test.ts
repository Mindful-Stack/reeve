// @vitest-environment jsdom
// web/src/lib/ui/Picker.svelte.test.ts
import { flushSync, mount, tick, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import Harness from './PickerHarness.test.svelte';

const options = [
	{ value: 'GRA-7', label: 'GRA-7 Migration findings' },
	{ value: 'GRA-8', label: 'GRA-8 Go-live' },
	{ value: 'GRA-608', label: 'GRA-608 Each person lands on what they need' }
];
let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});
function setup(start = '', empty?: string) {
	const spies = { onchange: vi.fn(), onclose: vi.fn() };
	app = mount(Harness, { target: document.body, props: { options, start, empty, ...spies } });
	flushSync();
	const input = document.body.querySelector<HTMLInputElement>('input[role="combobox"]')!;
	const key = (k: string, init: KeyboardEventInit = {}) => {
		const e = new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...init });
		input.dispatchEvent(e);
		flushSync();
		return e;
	};
	/** Types as a browser does into the focused input: a keydown per character, then the character replaces the
	 *  selection (jsdom does not edit the value on a key), then an `input` event. */
	const type = (t: string) => {
		for (const ch of t) {
			if (key(ch).defaultPrevented) continue;
			const v = input.value;
			const from = input.selectionStart ?? v.length;
			input.value = v.slice(0, from) + ch + v.slice(input.selectionEnd ?? v.length);
			input.setSelectionRange(from + 1, from + 1);
			input.dispatchEvent(new Event('input', { bubbles: true }));
			flushSync();
		}
	};
	const shown = () => [...document.body.querySelectorAll('[role="option"]')].map((o) => o.textContent?.trim());
	const bound = () => document.body.querySelector('[data-testid="bound"]')!.textContent;
	return { input, key, type, shown, bound, ...spies };
}

it('is a named combobox showing the current label while closed', () => {
	const { input } = setup('GRA-8');
	expect(input.getAttribute('aria-label')).toBe('Epic');
	expect(input.getAttribute('aria-expanded')).toBe('false');
	expect(input.value).toBe('GRA-8 Go-live');
});

it('typing into a picker focused by script replaces its label, also after Escape closed the list', async () => {
	const { input, type, key, shown } = setup('GRA-7');
	expect(document.activeElement).toBe(input); // the Dialog's focusFirst(), not a click or Tab
	type('go');
	expect(shown()).toEqual(['GRA-8 Go-live']);
	key('Escape');
	await tick();
	expect(input.value).toBe('GRA-7 Migration findings');
	type('each');
	expect(shown()).toEqual(['GRA-608 Each person lands on what they need']);
});

it('ArrowUp moves the active option back up, and stops at the first', () => {
	const { type, key, input } = setup('');
	type('gra');
	const active = () => document.getElementById(input.getAttribute('aria-activedescendant') ?? '')?.textContent?.trim();
	const first = active(); // typing makes the first match active
	key('ArrowDown');
	expect(active()).not.toBe(first);
	key('ArrowUp');
	expect(active()).toBe(first);
	key('ArrowUp');
	expect(active()).toBe(first);
});

it('narrows as you type and picks with the keyboard without submitting the form', () => {
	const { type, key, shown, bound, onchange, input } = setup('', 'No epic');
	type('live go');
	expect(shown()).toEqual(['GRA-8 Go-live']);
	key('ArrowDown');
	expect(input.getAttribute('aria-activedescendant')).toBeTruthy();
	const enter = key('Enter');
	expect(enter.defaultPrevented).toBe(true);
	expect(bound()).toBe('GRA-8');
	expect(onchange).toHaveBeenCalledWith('GRA-8');
	expect(input.value).toBe('GRA-8 Go-live');
});

it('offers the empty choice first while the query is blank', () => {
	const { key, shown } = setup('GRA-7', 'No epic');
	key('ArrowDown');
	expect(shown()[0]).toBe('No epic');
});

it('Escape while open closes it and is cancelled (every Escape listener skips a prevented event); while closed it is left alone', () => {
	const { type, key, input, shown } = setup('GRA-7');
	type('xyz');
	expect(shown()).toEqual([]);
	expect(document.body.querySelector('[role="status"]')!.textContent).toBe('No match');
	const esc = key('Escape');
	expect(esc.defaultPrevented).toBe(true);
	expect(input.getAttribute('aria-expanded')).toBe('false');
	expect(input.value).toBe('GRA-7 Migration findings');
	expect(key('Escape').defaultPrevented).toBe(false);
});

it('Enter that commits an IME composition picks nothing and keeps the list and the query', () => {
	const { type, key, shown, bound, input } = setup('', 'No epic');
	type('live go');
	const e = key('Enter', { isComposing: true });
	expect(e.defaultPrevented).toBe(false);
	expect(bound()).toBe('');
	expect(input.getAttribute('aria-expanded')).toBe('true');
	expect(shown()).toEqual(['GRA-8 Go-live']);
});

it('Enter with Ctrl, Meta or Alt picks nothing and is left to the page', () => {
	const { type, key, shown, bound, input } = setup('', 'No epic');
	type('live go');
	expect(shown()).toEqual(['GRA-8 Go-live']);
	for (const init of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }]) {
		const e = key('Enter', init);
		expect(e.defaultPrevented).toBe(false);
		expect(bound()).toBe('');
		expect(input.getAttribute('aria-expanded')).toBe('false'); // closed, so the list does not outlive the page's action
		input.value = '';
		type('live go'); // reopen for the next modified key
	}
	expect(key('Enter').defaultPrevented).toBe(true);
	expect(bound()).toBe('GRA-8');
});

it('Enter while closed is left to the form', () => {
	const { key } = setup('GRA-7');
	expect(key('Enter').defaultPrevented).toBe(false);
});

it('a mouse pick keeps focus and sets the value; blur drops free text', async () => {
	const { type, input, bound } = setup('GRA-7');
	type('608');
	const list = document.body.querySelector<HTMLElement>('[role="listbox"]')!;
	const down = new MouseEvent('mousedown', { bubbles: true, cancelable: true });
	list.dispatchEvent(down);
	expect(down.defaultPrevented).toBe(true);
	list.querySelector<HTMLElement>('[role="option"]')!.click();
	flushSync();
	expect(bound()).toBe('GRA-608');
	expect(input.getAttribute('aria-expanded')).toBe('false');
	await tick();
	type('nonsense');
	input.dispatchEvent(new FocusEvent('blur'));
	flushSync();
	expect(bound()).toBe('GRA-608');
	expect(input.value).toBe('GRA-608 Each person lands on what they need');
});

it('shows a current value the list does not hold', () => {
	const { input } = setup('GRA-999');
	expect(input.value).toBe('GRA-999');
});

it('a pick is a change the dialog sees: a backdrop click no longer closes it; picking the same value is none', () => {
	const { key, onclose } = setup('GRA-7');
	const changes = vi.fn();
	document.body.querySelector('form')!.addEventListener('change', changes);
	key('ArrowDown'); // opens on the current value
	key('Enter'); // picks it again: no change
	expect(changes).not.toHaveBeenCalled();
	key('ArrowDown');
	key('ArrowDown');
	key('Enter'); // no typing, so no `input` event either
	expect(changes).toHaveBeenCalledOnce();
	const d = document.body.querySelector('dialog')!;
	d.getBoundingClientRect = () => ({ left: 100, top: 100, right: 300, bottom: 300, width: 200, height: 200, x: 100, y: 100, toJSON() {} }) as DOMRect;
	d.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, clientX: 10, clientY: 10 }));
	d.dispatchEvent(new MouseEvent('click', { bubbles: true, clientX: 10, clientY: 10 }));
	expect(onclose).not.toHaveBeenCalled();
});
