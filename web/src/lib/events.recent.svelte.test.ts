import { describe, expect, it } from 'vitest';
import type { ActivityEntry } from '#lib/api/types.ts';
import { freshNote, RECENT_MS, type Arrival } from './events.svelte.ts';

const entry = (key: string, actor: string, action = 'moved') =>
	({ id: 1, at: '2026-11-01T10:00:00Z', entity_key: key, entity_type: 'story', actor, action, via: 'interactive', payload: {} }) satisfies ActivityEntry;

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
});
