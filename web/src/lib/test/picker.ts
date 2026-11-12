// web/src/lib/test/picker.ts: drive lib/ui/Picker.svelte from a jsdom page test, as a person would.
import { flushSync } from 'svelte';

/** The picker named `label` inside `root`. */
export function picker(root: ParentNode, label: string): HTMLInputElement {
	const el = root.querySelector<HTMLInputElement>(`input[role="combobox"][aria-label="${label}"]`);
	if (!el) throw new Error(`no picker named ${label}`);
	return el;
}

/** The open picker's options (none when it shows "No match"). */
function options(input: HTMLInputElement): HTMLElement[] {
	const list = document.getElementById(input.getAttribute('aria-controls')!);
	return list ? [...list.querySelectorAll<HTMLElement>('[role="option"]')] : [];
}

/** The picker a person can act on: a disabled one throws, as a page test must not act where a person cannot. */
function enabled(root: ParentNode, label: string): HTMLInputElement {
	const input = picker(root, label);
	if (input.disabled) throw new Error(`picker ${label} is disabled`);
	return input;
}

/** The labels the open picker offers. */
export function offered(root: ParentNode, label: string): string[] {
	const input = enabled(root, label);
	input.focus();
	input.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
	flushSync();
	const out = options(input).map((o) => o.textContent!.trim());
	input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
	flushSync();
	return out;
}

/** Opens the picker, types `query` and clicks the first option that is `optionText` (else one that starts with it and a space, so
 *  `E-2` never picks `E-20 …`). The whole value is
 *  replaced at once, as typing over the label the picker selects on focus does (Picker.svelte.test.ts proves that). */
export function choose(root: ParentNode, label: string, query: string, optionText = query): void {
	const input = enabled(root, label);
	input.focus();
	input.value = query;
	input.dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
	const texts = options(input).map((o) => [o, o.textContent!.trim()] as const);
	const opt = (texts.find(([, t]) => t === optionText) ?? texts.find(([, t]) => t.startsWith(optionText + ' ')))?.[0];
	if (!opt) throw new Error(`no option ${optionText} in ${label}`);
	opt.click();
	flushSync();
}
