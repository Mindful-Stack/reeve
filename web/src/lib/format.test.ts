import { describe, expect, it } from 'vitest';
import { age, errorMessage, month, short } from './format.ts';

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
