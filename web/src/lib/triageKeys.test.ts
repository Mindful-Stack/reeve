// web/src/lib/triageKeys.test.ts
import { describe, expect, it } from 'vitest';
import { command, type Command, type Key, type KeyContext } from './triageKeys.ts';

const ctx = (over: Partial<KeyContext> = {}): KeyContext => ({ typing: false, inList: false, overlay: false, ...over });
const key = (k: string, over: Partial<Key> = {}): Key => ({
	key: k, ctrlKey: false, metaKey: false, altKey: false, repeat: false, isComposing: false, defaultPrevented: false, ...over
});

describe('command', () => {
	it('maps every key', () => {
		const map: Record<string, Command> = {
			j: { kind: 'next' }, k: { kind: 'prev' }, e: { kind: 'epic' }, t: { kind: 'kind' }, c: { kind: 'criteria' },
			d: { kind: 'drop' }, s: { kind: 'skip' }, o: { kind: 'open' }, u: { kind: 'undo' }, '3': { kind: 'duplicate', n: 3 }
		};
		for (const [k, want] of Object.entries(map)) expect(command(key(k), ctx()), k).toEqual(want);
		expect(command(key('?'), ctx())).toEqual({ kind: 'help' });
		expect(command(key('6'), ctx())).toBeNull();
		expect(command(key('0'), ctx())).toBeNull();
		expect(command(key('1'), ctx())).toEqual({ kind: 'duplicate', n: 1 });
		expect(command(key('5'), ctx())).toEqual({ kind: 'duplicate', n: 5 });
	});
	it('Cmd or Ctrl or Alt with a letter does nothing', () => {
		for (const over of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }]) expect(command(key('j', over), ctx()), JSON.stringify(over)).toBeNull();
	});
	it('arrows move only in the queue list', () => {
		expect(command(key('ArrowDown'), ctx({ inList: true }))).toEqual({ kind: 'next' });
		expect(command(key('ArrowUp'), ctx({ inList: true }))).toEqual({ kind: 'prev' });
		expect(command(key('ArrowDown'), ctx())).toBeNull();
	});
	it('while typing only Ctrl/Cmd+Enter and Escape act', () => {
		expect(command(key('d'), ctx({ typing: true }))).toBeNull();
		expect(command(key('Enter', { ctrlKey: true }), ctx({ typing: true }))).toEqual({ kind: 'place' });
		expect(command(key('Enter', { metaKey: true }), ctx({ typing: true }))).toEqual({ kind: 'place' });
		expect(command(key('Escape'), ctx({ typing: true }))).toEqual({ kind: 'escape' });
		expect(command(key('Enter'), ctx({ typing: true }))).toBeNull();
	});
	it('nothing acts on a repeat, during composition, after another handler, with a drawer open, or with a modifier', () => {
		expect(command(key('d', { repeat: true }), ctx())).toBeNull();
		expect(command(key('d', { isComposing: true }), ctx())).toBeNull();
		expect(command(key('d', { defaultPrevented: true }), ctx())).toBeNull();
		expect(command(key('d'), ctx({ overlay: true }))).toBeNull();
		expect(command(key('Enter', { ctrlKey: true }), ctx({ overlay: true }))).toBeNull();
		expect(command(key('d', { ctrlKey: true }), ctx())).toBeNull();
		expect(command(key('j', { altKey: true }), ctx())).toBeNull();
		expect(command(key('Enter', { ctrlKey: true, altKey: true }), ctx())).toBeNull();
	});
});
