import { describe, expect, it } from 'vitest';
import { keyed, narrow, withCurrent, type PickerOption } from './picker.ts';

const opts: PickerOption[] = [
	{ value: 'GRA-7', label: 'GRA-7 Migration findings' },
	{ value: 'GRA-608', label: 'GRA-608 Each person lands on what they need' },
	{ value: 'GRA-8', label: 'GRA-8 Go-live' }
];

describe('narrow', () => {
	it('keeps every option for an empty or blank query', () => {
		expect(narrow(opts, '  ')).toEqual(opts);
	});
	it('matches every word, in any order, ignoring case', () => {
		expect(narrow(opts, 'live go').map((o) => o.value)).toEqual(['GRA-8']);
		expect(narrow(opts, 'MIGRATION').map((o) => o.value)).toEqual(['GRA-7']);
	});
	it('matches a key fragment', () => {
		expect(narrow(opts, 'gra-60').map((o) => o.value)).toEqual(['GRA-608']);
	});
});

describe('withCurrent', () => {
	it('adds a current value the list does not hold, labelled by itself', () => {
		expect(withCurrent(opts, 'GRA-999').at(-1)).toEqual({ value: 'GRA-999', label: 'GRA-999' });
		expect(withCurrent(opts, 'GRA-7')).toBe(opts);
		expect(withCurrent(opts, '')).toBe(opts);
	});
});

describe('keyed', () => {
	it('labels an item by its key and title, valued by its key', () => {
		expect(keyed([{ key: 'GRA-8', title: 'Go-live' }])).toEqual([{ value: 'GRA-8', label: 'GRA-8 Go-live' }]);
	});
});
