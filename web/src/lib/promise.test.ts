import { describe, expect, it } from 'vitest';
import type { EpicPromise, PromiseDue, PromiseFact, PromiseState, PromisedFilter, SprintPromise, StoryPromise } from '#lib/api/types.ts';
import words from './promise.fixture.json';
import { PROMISED_OPTIONS, chip, epicNote, inheritedEpic, isPromisedFilter, lineParts, promiseActions, promisedMatches, sprintRowParts, type PromisePart } from './promise.ts';

const fact = (on: string, to: string | null = 'Acme', by = 'Dana'): PromiseFact => ({ on, to, by, at: '2026-11-01T09:00:00Z' });

function sp(over: Partial<StoryPromise> = {}): StoryPromise {
	return { own: fact('2026-11-20'), epic: null, on: '2026-11-20', holder: 'own', state: 'open', due: 'later', days: null, ...over };
}

/** The words of parts, as a host renders them (an epic key as its text). */
const joined = (parts: PromisePart[]) => parts.map((x) => ('text' in x ? x.text : x.epic)).join('');

// The word rows live in promise.fixture.json, which the CLI's tests (crates/reeve/src/promise.rs) read too; the filter
// rows are read by the domain's tests too (crates/reeve-planning/src/domain/promise.rs).
type Row<P> = { label: string; promise: P; text: string };
const chipRows = words.chip as unknown as (Row<StoryPromise> & { via?: boolean })[];
const epicChipRows = words.epicChip as unknown as Row<EpicPromise>[];
const lineRows = words.line as unknown as Row<StoryPromise>[];
const epicLineRows = words.epicLine as unknown as Row<EpicPromise>[];
const sprintRows = words.sprintRow as unknown as Row<SprintPromise>[];
const noteRows = words.epicNote as { label: string; epic: { key: string; on: string }; chosen: string; text: string }[];
const filterRows = words.filter as { label: string; state: PromiseState | null; due: PromiseDue | null; matches: Record<PromisedFilter, boolean> }[];

describe('chip', () => {
	it.each(chipRows.map((r) => [r.label, r] as const))('%s', (_, r) =>
		expect(joined(chip(r.promise, r.via === undefined ? {} : { via: r.via }).parts)).toBe(r.text)
	);

	it('keeps the epic key a part of its own, so the chip can keep it on one line', () => {
		const inherited = sp({ own: null, epic: { key: 'ABC-7', fact: fact('2026-11-20') }, holder: 'epic', due: 'later' });
		expect(chip(inherited).parts).toEqual([{ text: 'Promised by 20 Nov' }, { text: ' · via ' }, { epic: 'ABC-7' }]);
		expect(chip(inherited, { via: false }).parts).toEqual([{ text: 'Promised by 20 Nov' }]);
	});
});

describe('an epic chip', () => {
	it.each(epicChipRows.map((r) => [r.label, r] as const))('%s', (_, r) => expect(joined(chip(r.promise).parts)).toBe(r.text));
});

describe('a sprint row', () => {
	it.each(sprintRows.map((r) => [r.label, r] as const))('%s', (_, r) => expect(joined(sprintRowParts(r.promise))).toBe(r.text));
});

describe('tone and icon', () => {
	it.each<[PromiseState | null, string, string | null]>([
		['overdue', 'warn', 'clock'],
		['kept', 'ready', 'check'],
		['late', 'muted', null],
		['dropped', 'muted', null],
		['open', 'neutral', 'calendar'],
		['unknown', 'neutral', 'calendar'],
		[null, 'neutral', 'calendar']
	])('a story %s is %s with %s', (state, tone, icon) => expect(chip(sp({ state }))).toMatchObject({ tone, icon }));

	it('an epic chip warns with the clock once past, and is neutral before', () => {
		expect(chip({ fact: fact('2026-11-20'), due: null, past: true })).toMatchObject({ tone: 'warn', icon: 'clock' });
		expect(chip({ fact: fact('2026-11-20'), due: 'later', past: false })).toMatchObject({ tone: 'neutral', icon: 'calendar' });
	});
});

describe('lineParts', () => {
	it.each(lineRows.map((r) => [r.label, r] as const))('%s', (_, r) => expect(joined(lineParts(r.promise))).toBe(r.text));
	it.each(epicLineRows.map((r) => [`epic, ${r.label}`, r] as const))('%s', (_, r) => expect(joined(lineParts(r.promise))).toBe(r.text));
	it('links the epic key as a part', () => {
		const both = sp({ own: fact('2026-11-30', 'Beta'), epic: { key: 'ABC-7', fact: fact('2026-11-18') }, holder: 'epic', on: '2026-11-18' });
		expect(lineParts(both).filter((x) => 'epic' in x)).toEqual([{ epic: 'ABC-7' }]);
	});
});

describe('epicNote', () => {
	it.each(noteRows.map((r) => [r.label, r] as const))('%s', (_, r) => expect(epicNote(r.epic, r.chosen)).toBe(r.text));

	it('inheritedEpic is set whenever the story inherits an epic promise, whoever holds the date', () => {
		const e = { key: 'ABC-7', fact: fact('2026-11-30') };
		expect(inheritedEpic(sp({ epic: e, holder: 'own' }))).toEqual({ key: 'ABC-7', on: '2026-11-30' });
		expect(inheritedEpic(sp({ epic: e, holder: 'epic' }))).toEqual({ key: 'ABC-7', on: '2026-11-30' });
		expect(inheritedEpic(sp({}))).toBeNull();
		expect(inheritedEpic(null)).toBeNull();
	});
});

describe('promiseActions', () => {
	it.each<[boolean, boolean, [string, string][]]>([
		[false, false, [['Promise by…', 'set']]],
		[true, false, [['Change promise…', 'set'], ['Clear promise', 'clear']]],
		[false, true, []],
		[true, true, [['Clear promise', 'clear']]]
	])('own %s, done %s', (own, done, items) => expect(promiseActions(own, done).map((a) => [a.label, a.kind])).toEqual(items));
});

// The fixture's filter rows, which the domain's `Filter::matches` test reads too.
describe('promisedMatches', () => {
	const filters: PromisedFilter[] = ['this_sprint', 'next_sprint', 'overdue', 'any'];
	it.each(filterRows.flatMap((r) => filters.map((f) => [r.label, f, r] as const)))('%s / %s', (_, f, r) =>
		expect(promisedMatches(f, { state: r.state, due: r.due })).toBe(r.matches[f])
	);
	it('has a column for every filter in every row', () => {
		for (const r of filterRows) expect(Object.keys(r.matches).sort()).toEqual([...filters].sort());
	});
});

describe('the Promise filter', () => {
	it('names its options in order, the empty one meaning no filter', () => {
		expect(PROMISED_OPTIONS).toEqual([
			{ value: 'any', label: 'Promised (open or overdue)' },
			{ value: 'this_sprint', label: 'Due this sprint, incl. overdue' },
			{ value: 'next_sprint', label: 'Due next sprint' },
			{ value: 'overdue', label: 'Overdue' }
		]);
	});
	it('accepts only its four values', () => {
		expect(['any', 'this_sprint', 'next_sprint', 'overdue'].every(isPromisedFilter)).toBe(true);
		expect([null, undefined, '', 'bogus', 'later'].some(isPromisedFilter)).toBe(false);
	});
});
