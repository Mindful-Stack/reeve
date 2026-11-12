import { describe, expect, it } from 'vitest';
import { age, dayLabel, errorMessage, month, short } from './format.ts';

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
	it('names the file of an attachment entry and spaces every other action', async () => {
		const { actionLabel } = await import('./format.ts');
		expect(actionLabel('attachment_added', { attachment_id: 'x', filename: 'shot.png' })).toBe('added attachment shot.png');
		expect(actionLabel('attachment_removed', { filename: 'shot.png' })).toBe('deleted attachment shot.png');
		expect(actionLabel('attachment_removed', null)).toBe('deleted attachment');
		expect(actionLabel('goal_linked', null)).toBe('goal linked');
	});

	// The account activity page's label() delegates to actionLabel, so these are its words too (it has no test file).
	const promised = (on: string, to: string | null, previous: { on: string; to: string | null } | null) => ({ item_type: 'story', on, to, previous });
	it.each<[string, string, unknown, string, string]>([
		['a new promise', 'promised', promised('2026-11-20', 'Acme', null), 'promised by 20 Nov to Acme', 'promised ABC-12 by 20 Nov to Acme'],
		['a new promise, no recipient', 'promised', promised('2026-11-20', null, null), 'promised by 20 Nov', 'promised ABC-12 by 20 Nov'],
		[
			'a new date',
			'promised',
			promised('2026-11-27', 'Acme', { on: '2026-11-20', to: 'Acme' }),
			'changed the promise to 27 Nov',
			'changed the promise on ABC-12 to 27 Nov'
		],
		[
			'a new date and recipient',
			'promised',
			promised('2026-11-27', 'Beta', { on: '2026-11-20', to: 'Acme' }),
			'changed the promise to 27 Nov to Beta',
			'changed the promise on ABC-12 to 27 Nov to Beta'
		],
		[
			'a new date, recipient removed',
			'promised',
			promised('2026-11-27', null, { on: '2026-11-20', to: 'Acme' }),
			'changed the promise to 27 Nov and removed its recipient',
			'changed the promise on ABC-12 to 27 Nov and removed its recipient'
		],
		[
			'a new recipient',
			'promised',
			promised('2026-11-20', 'Beta', { on: '2026-11-20', to: 'Acme' }),
			"changed the promise's recipient to Beta",
			'changed the recipient of the promise on ABC-12 to Beta'
		],
		[
			'a first recipient',
			'promised',
			promised('2026-11-20', 'Beta', { on: '2026-11-20', to: null }),
			"changed the promise's recipient to Beta",
			'changed the recipient of the promise on ABC-12 to Beta'
		],
		[
			'the recipient removed',
			'promised',
			promised('2026-11-20', null, { on: '2026-11-20', to: 'Acme' }),
			"removed the promise's recipient",
			'removed the recipient of the promise on ABC-12'
		],
		[
			'nothing changed',
			'promised',
			promised('2026-11-20', 'Acme', { on: '2026-11-20', to: 'Acme' }),
			'promised by 20 Nov to Acme',
			'promised ABC-12 by 20 Nov to Acme'
		],
		['cleared', 'unpromised', { item_type: 'story', previous: { on: '2026-11-20', to: 'Acme' } }, 'cleared the promise', 'cleared the promise on ABC-12'],
		['an unreadable promise', 'promised', null, 'promised', 'promised ABC-12'],
		['an unreadable clear', 'unpromised', null, 'cleared the promise', 'cleared the promise on ABC-12']
	])('%s', async (_, action, payload, plain, keyed) => {
		const { actionLabel } = await import('./format.ts');
		expect(actionLabel(action, payload)).toBe(plain);
		expect(actionLabel(action, payload, 'ABC-12')).toBe(keyed);
	});
	it('names its own words for attachments and promises only', async () => {
		const { hasOwnWords } = await import('./format.ts');
		expect(['attachment_added', 'attachment_removed', 'promised', 'unpromised'].every(hasOwnWords)).toBe(true);
		expect(['moved', 'goal_linked', 'override'].some(hasOwnWords)).toBe(false);
	});
});
