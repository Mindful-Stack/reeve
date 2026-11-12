// web/src/lib/triageKeys.ts: a keydown on the triage area to a command. The page listens on its own container only (the
// keys are active only while focus is there, WCAG 2.1.4), and passes what it knows about focus and the drawer.
export type Command =
	| { kind: 'next' | 'prev' | 'epic' | 'kind' | 'criteria' | 'place' | 'drop' | 'skip' | 'open' | 'undo' | 'escape' | 'help' }
	| { kind: 'duplicate'; n: number };

export type KeyContext = {
	/** Focus is in an enabled text field, select or contenteditable (the Picker's input counts). */
	typing: boolean;
	/** Focus is on the queue listbox. */
	inList: boolean;
	/** A story or decision drawer is open over the page. (A dialog, a menu or an open Picker list never lets a key reach
	 *  the area: a modal dialog makes it inert, a menu holds focus, the Picker stops Escape and takes Enter.) */
	overlay: boolean;
};

/** What `command` reads of a keydown. Shift is not read: `?` is typed with it on most layouts. */
export type Key = Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'repeat' | 'isComposing' | 'defaultPrevented'>;

const LETTERS: Record<string, Command> = {
	j: { kind: 'next' }, k: { kind: 'prev' }, e: { kind: 'epic' }, t: { kind: 'kind' }, c: { kind: 'criteria' },
	d: { kind: 'drop' }, s: { kind: 'skip' }, o: { kind: 'open' }, u: { kind: 'undo' }, '?': { kind: 'help' }
};

export function command(e: Key, c: KeyContext): Command | null {
	if (c.overlay || e.defaultPrevented || e.repeat || e.isComposing) return null;
	if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.altKey) return { kind: 'place' };
	if (e.ctrlKey || e.metaKey || e.altKey) return null;
	if (e.key === 'Escape') return { kind: 'escape' };
	if (c.typing) return null;
	if (c.inList && e.key === 'ArrowDown') return { kind: 'next' };
	if (c.inList && e.key === 'ArrowUp') return { kind: 'prev' };
	if (/^[1-5]$/.test(e.key)) return { kind: 'duplicate', n: Number(e.key) };
	return LETTERS[e.key] ?? null;
}
