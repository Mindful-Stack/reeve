import { describe, expect, it } from 'vitest';
import { age, dateLabel, dayLabel, errorMessage, month, short } from './format.ts';
import words from './promise.fixture.json';

const now = Date.parse('2026-10-26T12:00:00Z');
const ago = (days: number) => new Date(now - days * 86_400_000).toISOString();

describe('age', () => {
	const minsAgo = (m: number) => new Date(now - m * 60_000).toISOString();
	it.each([
		[0, '0m'],
		[1, '1m'],
		[59, '59m'],
		[60, '1h'],
		[61, '1h'],
		[23 * 60 + 59, '23h'],
		[24 * 60, '1d']
	])('%i minutes ago is %s', (mins, expected) => expect(age(minsAgo(mins), now)).toBe(expected));

	it.each([
		[1, '1d'],
		[3, '3d'],
		[13, '13d'],
		[14, '2w'],
		[14 * 1, '2w'],
		[59, '8w'],
		[60, '2mo'],
		[150, '5mo']
	])('%i days ago is %s', (days, expected) => expect(age(ago(days), now)).toBe(expected));

	it('clamps a future date to 0m', () => expect(age(ago(-5), now)).toBe('0m'));
	it('returns "?" for an invalid date', () => expect(age('not a date', now)).toBe('?'));
});

describe('month', () => {
	it('formats year-month', () => expect(month('2026-11')).toBe('Nov 2026'));
	it('returns anything else unchanged', () => {
		expect(month('2026-13')).toBe('2026-13');
		expect(month('soon')).toBe('soon');
	});
});

describe('dayLabel', () => {
	// Built from the string's parts, so no time zone can shift the day.
	it('formats a date as day and month', () => expect(dayLabel('2026-11-20')).toBe('20 Nov'));
	it('drops the leading zero', () => expect(dayLabel('2026-01-01')).toBe('1 Jan'));
	it('returns anything else unchanged', () => {
		expect(dayLabel('2026-13-01')).toBe('2026-13-01');
		expect(dayLabel('2026-11')).toBe('2026-11');
		expect(dayLabel('soon')).toBe('soon');
	});
});

describe('dateLabel', () => {
	it('adds the year to the day label', () => expect(dateLabel('2026-01-01')).toBe('1 Jan 2026'));
	it('returns anything else unchanged', () => expect(dateLabel('2026-13-01')).toBe('2026-13-01'));
});

describe('short', () => {
	it('takes the local part', () => expect(short('dan@example.com')).toBe('dan'));
	it('leaves a non-address unchanged', () => expect(short('dan')).toBe('dan'));
});

describe('errorMessage', () => {
	it('shows an Error by its message and anything else as text', () => {
		expect(errorMessage(new Error('boom'))).toBe('boom');
		expect(errorMessage('plain')).toBe('plain');
		expect(errorMessage(42)).toBe('42');
	});
});

describe('actionLabel', () => {
	it('names the file of an attachment entry', async () => {
		const { actionLabel } = await import('./format.ts');
		expect(actionLabel('attachment_added', { attachment_id: 'x', filename: 'shot.png' })).toBe('added attachment shot.png');
		expect(actionLabel('attachment_removed', { filename: 'shot.png' })).toBe('deleted attachment shot.png');
		expect(actionLabel('attachment_removed', null)).toBe('deleted attachment');
	});

	// The account activity page's label() delegates to actionLabel, so these are its words too (it has no test file). The
	// rows live in promise.fixture.json, which the TUI ticker's tests (crates/reeve/src/promise.rs) read too.
	const activityRows = words.activity as { label: string; action: string; payload: unknown; plain: string; keyed: string }[];
	it.each<[string, string, unknown, string, string]>(activityRows.map((r) => [r.label, r.action, r.payload, r.plain, r.keyed]))('%s', async (_, action, payload, plain, keyed) => {
		const { actionLabel } = await import('./format.ts');
		expect(actionLabel(action, payload)).toBe(plain);
		expect(actionLabel(action, payload, 'ABC-12')).toBe(keyed);
	});
	it('has no words for any other action, which reads as plainAction', async () => {
		const { actionLabel, plainAction } = await import('./format.ts');
		for (const a of ['moved', 'goal_linked', 'override']) expect(actionLabel(a, null, 'ABC-12')).toBeNull();
		expect(plainAction('goal_linked')).toBe('goal linked');
	});
});

describe('entryAction', () => {
	it('adds the client a write was made through, to worded and plain actions alike', async () => {
		const { entryAction } = await import('./format.ts');
		expect(entryAction('moved', { from: 'todo', to: 'in_progress', client: 'claude.ai' })).toBe('moved via claude.ai');
		expect(entryAction('goal_linked', { client: 'chatgpt.com' })).toBe('goal linked via chatgpt.com');
		expect(entryAction('attachment_added', { filename: 'shot.png', client: 'claude.ai' })).toBe('added attachment shot.png via claude.ai');
	});
	it('reads as before without a client', async () => {
		const { entryAction } = await import('./format.ts');
		expect(entryAction('moved', { from: 'todo', to: 'in_progress' })).toBe('moved');
		expect(entryAction('attachment_added', { filename: 'shot.png' })).toBe('added attachment shot.png');
		for (const payload of [null, undefined, 'text', {}, { client: '' }, { client: 7 }, { client: null }]) expect(entryAction('edited', payload)).toBe('edited');
	});
});
