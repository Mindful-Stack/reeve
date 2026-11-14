import { describe, expect, it } from 'vitest';
import type { ActivityEntry } from '#lib/api/types.ts';
import { freshNote, RECENT_MS, type Arrival } from './events.svelte.ts';

const entry = (key: string, actor: string, action = 'moved', payload: unknown = {}) =>
	({ id: 1, at: '2026-11-01T10:00:00Z', entity_key: key, entity_type: 'story', actor, action, via: 'interactive', payload }) satisfies ActivityEntry;

describe('freshNote', () => {
	const now = 1_000_000;
	const recent: Arrival[] = [{ entry: entry('ABC-1', 'erik@example.com'), arrived: now - 2_000 }];

	it('names another actor and the action for a key changed in the window', () => {
		expect(freshNote(recent, 'ABC-1', 'anna@example.com', now)).toBe('erik moved this · just now');
	});
	it('says nothing for my own change, another key, or after the window', () => {
		expect(freshNote(recent, 'ABC-1', 'erik@example.com', now)).toBeNull();
		expect(freshNote(recent, 'ABC-2', 'anna@example.com', now)).toBeNull();
		expect(freshNote(recent, 'ABC-1', 'anna@example.com', now - 2_000 + RECENT_MS)).toBeNull();
	});
	it('uses the latest arrival for a key and spaces out underscores', () => {
		const two: Arrival[] = [...recent, { entry: entry('ABC-1', 'sara@example.com', 'criteria_edited'), arrived: now - 1_000 }];
		expect(freshNote(two, 'ABC-1', 'anna@example.com', now)).toBe('sara criteria edited this · just now');
	});
	it('words a promise with the key, never "promise cleared this"', () => {
		const at = (action: string, payload: unknown): Arrival[] => [{ entry: entry('ABC-12', 'erik@example.com', action, payload), arrived: now - 1_000 }];
		const note = (action: string, payload: unknown) => freshNote(at(action, payload), 'ABC-12', 'anna@example.com', now);
		expect(note('promised', { item_type: 'story', on: '2026-11-20', to: 'Acme', previous: null })).toBe('erik promised ABC-12 by 20 Nov 2026 to Acme · just now');
		expect(note('promised', { item_type: 'story', on: '2026-11-27', to: 'Acme', previous: { on: '2026-11-20', to: 'Acme' } })).toBe(
			'erik changed the promise on ABC-12 to 27 Nov 2026 · just now'
		);
		const cleared = note('promise_cleared', { item_type: 'story', previous: { on: '2026-11-20', to: null } });
		expect(cleared).toBe('erik cleared the promise on ABC-12 · just now');
		expect(cleared).not.toContain('promise cleared this');
	});
	it('says which client the change was made through, after the words and before the age', () => {
		const through = (action: string, payload: unknown): Arrival[] => [{ entry: entry('ABC-12', 'erik@example.com', action, payload), arrived: now - 1_000 }];
		const note = (action: string, payload: unknown) => freshNote(through(action, payload), 'ABC-12', 'anna@example.com', now);
		expect(note('moved', { from: 'todo', to: 'in_progress', client: 'claude.ai' })).toBe('erik moved this via claude.ai · just now');
		expect(note('promise_cleared', { item_type: 'story', previous: { on: '2026-11-20', to: null }, client: 'claude.ai' })).toBe(
			'erik cleared the promise on ABC-12 via claude.ai · just now'
		);
		expect(note('moved', { from: 'todo', to: 'in_progress', client: '' })).toBe('erik moved this · just now');
	});
});
