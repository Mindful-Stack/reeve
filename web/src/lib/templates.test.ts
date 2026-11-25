import { describe, expect, it } from 'vitest';
import { defaultOf, pick, untouched } from './templates.ts';
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
