import { describe, expect, it } from 'vitest';
import { defaultOf, pick, startedFrom, untouched } from './templates.ts';
import { briefTemplate } from '#lib/test/fixtures.ts';

describe('untouched', () => {
	it.each([
		[{ text: '', applied: '' }, true],
		[{ text: ' \n', applied: '# Scenario\n' }, true], // emptied by hand: nothing to lose
		[{ text: '# Scenario\n', applied: '# Scenario\n' }, true], // still exactly what the picker put there
		[{ text: '# Scenario\nmine', applied: '# Scenario\n' }, false],
		[{ text: '# Scenario', applied: '# Scenario\n' }, false], // one character removed is a change
		[{ text: 'Mine', applied: '' }, false]
	])('%j is %s', (filled, want) => expect(untouched(filled)).toBe(want));
});

describe('pick', () => {
	it('fills an untouched field with the chosen body, and remembers it as applied', () => {
		expect(pick({ text: '', applied: '' }, '## Debt\n')).toEqual({ kind: 'fill', text: '## Debt\n', applied: '## Debt\n' });
		expect(pick({ text: '# Scenario\n', applied: '# Scenario\n' }, '## Debt\n')).toEqual({ kind: 'fill', text: '## Debt\n', applied: '## Debt\n' });
	});
	it('empties an untouched field for "No template"', () => {
		expect(pick({ text: '# Scenario\n', applied: '# Scenario\n' }, '')).toEqual({ kind: 'fill', text: '', applied: '' });
	});
	it('asks before replacing text the author changed, whatever is chosen', () => {
		const typed = { text: '# Scenario\nmine', applied: '# Scenario\n' };
		expect(pick(typed, '## Debt\n')).toEqual({ kind: 'ask' });
		expect(pick(typed, '')).toEqual({ kind: 'ask' });
		expect(pick({ text: 'Mine', applied: '' }, '# Scenario\n')).toEqual({ kind: 'ask' });
	});
});

describe('defaultOf', () => {
	it('is the template the server marked, or none', () => {
		const marked = briefTemplate({ id: 2, name: 'Tech debt', is_default: true });
		expect(defaultOf([briefTemplate(), marked])).toBe(marked);
		expect(defaultOf([briefTemplate()])).toBeUndefined();
		expect(defaultOf([])).toBeUndefined();
	});
});

describe('startedFrom', () => {
	const scenario = briefTemplate({ id: 1, name: 'Scenario', body: '# Scenario\n' });
	const debt = briefTemplate({ id: 2, name: 'Tech debt', body: '## Debt\n', is_default: true });
	it('is the template whose body the text is, exactly', () => {
		expect(startedFrom([scenario, debt], '# Scenario\n')).toBe(scenario);
		expect(startedFrom([scenario, debt], '## Debt\n')).toBe(debt);
	});
	it('is none for text the author changed, for a blank text and for no templates', () => {
		expect(startedFrom([scenario, debt], '# Scenario\nmine')).toBeUndefined();
		expect(startedFrom([scenario, debt], '# Scenario')).toBeUndefined(); // one character short
		expect(startedFrom([scenario, debt], '')).toBeUndefined();
		expect(startedFrom([], '# Scenario\n')).toBeUndefined();
	});
	it('is the default when two templates share the body, wherever it is listed', () => {
		const twin = briefTemplate({ id: 3, name: 'Also debt', body: '## Debt\n' });
		expect(startedFrom([twin, debt], '## Debt\n')).toBe(debt);
		expect(startedFrom([debt, twin], '## Debt\n')).toBe(debt);
		// No default among the twins: the first listed.
		const other = briefTemplate({ id: 4, name: 'Third', body: '# Scenario\n' });
		expect(startedFrom([scenario, other], '# Scenario\n')).toBe(scenario);
	});
});
