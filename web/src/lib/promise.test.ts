import { describe, expect, it } from 'vitest';
import type { EpicPromise, PromiseDue, PromiseFact, PromiseState, PromisedFilter, StoryPromise } from '#lib/api/types.ts';
import { chip, chipText, epicChipText, epicLineText, epicNote, lineText, look, promiseActions, promisedMatches } from './promise.ts';

const fact = (on: string, to: string | null = 'Acme', by = 'Dana'): PromiseFact => ({ on, to, by, at: '2026-11-01T09:00:00Z' });

function sp(over: Partial<StoryPromise> = {}): StoryPromise {
	return { own: fact('2026-11-20'), epic: null, on: '2026-11-20', holder: 'own', state: 'open', due: 'later', days: null, ...over };
}

describe('chipText', () => {
	it.each<[string, Partial<StoryPromise>, string]>([
		['open, this sprint', { due: 'this_sprint' }, 'Promised 20 Nov · this sprint'],
		['open, next sprint', { due: 'next_sprint' }, 'Promised 20 Nov · next sprint'],
		['open, later', { due: 'later' }, 'Promised 20 Nov'],
		['overdue', { state: 'overdue', due: null, days: 3 }, 'Overdue 3 d · 20 Nov'],
		['kept', { state: 'kept', due: null }, 'Kept · 20 Nov'],
		['late', { state: 'late', due: null, days: 3 }, 'Done 3 d late · 20 Nov'],
		['dropped', { state: 'dropped', due: null }, 'Dropped · 20 Nov'],
		['state null', { state: null, due: null }, 'Promised 20 Nov']
	])('%s', (_, over, text) => expect(chipText(sp(over))).toBe(text));

	it('names the epic when the epic holds the date', () => {
		const p = sp({ own: null, epic: { key: 'ABC-7', fact: fact('2026-11-20') }, holder: 'epic', due: 'this_sprint' });
		expect(chipText(p)).toBe('Promised 20 Nov · this sprint · via ABC-7');
	});
});

describe('epicChipText', () => {
	const ep = (over: Partial<EpicPromise>): EpicPromise => ({ fact: fact('2026-11-20'), due: null, past: false, ...over });
	it.each<[Partial<EpicPromise>, string]>([
		[{ due: 'this_sprint' }, 'Promised 20 Nov · this sprint'],
		[{ due: 'next_sprint' }, 'Promised 20 Nov · next sprint'],
		[{ due: 'later' }, 'Promised 20 Nov'],
		[{ past: true }, 'Past 20 Nov']
	])('%o', (over, text) => expect(epicChipText(ep(over))).toBe(text));
});

describe('tone and icon', () => {
	it.each<[PromiseState | 'past' | null, string, string | null]>([
		['overdue', 'warn', 'clock'],
		['kept', 'ready', 'check'],
		['late', 'muted', null],
		['dropped', 'muted', null],
		['past', 'muted', null],
		['open', 'neutral', 'calendar'],
		[null, 'neutral', 'calendar']
	])('%s is %s with %s', (state, tone, icon) => expect(look(state)).toEqual({ tone, icon }));

	it('an epic chip is past or neutral', () => {
		expect(chip({ fact: fact('2026-11-20'), due: null, past: true })).toEqual({ text: 'Past 20 Nov', tone: 'muted', icon: null });
		expect(chip({ fact: fact('2026-11-20'), due: 'later', past: false }).tone).toBe('neutral');
		expect(chip(sp({ state: 'overdue', due: null, days: 3 }))).toEqual({ text: 'Overdue 3 d · 20 Nov', tone: 'warn', icon: 'clock' });
	});
});

describe('lineText', () => {
	it.each<[string, Partial<StoryPromise>, string]>([
		['own only', {}, 'Promised 20 Nov to Acme · set by Dana'],
		['own only, no recipient', { own: fact('2026-11-20', null) }, 'Promised 20 Nov · set by Dana'],
		[
			'epic only',
			{ own: null, epic: { key: 'ABC-7', fact: fact('2026-11-20') }, holder: 'epic' },
			'Promised 20 Nov via epic ABC-7 to Acme · set by Dana'
		],
		[
			'both, epic earlier',
			{ own: fact('2026-11-30', 'Beta'), epic: { key: 'ABC-7', fact: fact('2026-11-18') }, on: '2026-11-18', holder: 'epic' },
			'Promised 30 Nov to Beta. Epic ABC-7 is promised 18 Nov; the earlier date counts.'
		],
		[
			'both, own earlier',
			{ own: fact('2026-11-18', 'Beta'), epic: { key: 'ABC-7', fact: fact('2026-11-30') }, on: '2026-11-18', holder: 'own' },
			'Promised 18 Nov to Beta. Epic ABC-7 is promised 30 Nov.'
		],
		['overdue', { state: 'overdue', due: null, days: 3 }, 'Promised 20 Nov to Acme · set by Dana · overdue 3 d'],
		['kept', { state: 'kept', due: null }, 'Promised 20 Nov to Acme · set by Dana · kept'],
		['late', { state: 'late', due: null, days: 3 }, 'Promised 20 Nov to Acme · set by Dana · done 3 d late'],
		['dropped', { state: 'dropped', due: null }, 'Promised 20 Nov to Acme · set by Dana · dropped']
	])('%s', (_, over, text) => expect(lineText(sp(over))).toBe(text));

	it('shortens an email to its local part', () => {
		expect(lineText(sp({ own: fact('2026-11-20', 'Acme', 'dana@example.com') }))).toBe('Promised 20 Nov to Acme · set by dana');
	});
});

describe('epicLineText', () => {
	it('names the recipient and the setter, and says past once the date is', () => {
		expect(epicLineText({ fact: fact('2026-11-20'), due: 'later', past: false })).toBe('Promised 20 Nov to Acme · set by Dana');
		expect(epicLineText({ fact: fact('2026-11-20'), due: null, past: true })).toBe('Promised 20 Nov to Acme · set by Dana · past');
	});
});

describe('epicNote', () => {
	it('is set only when the epic holds the date', () => {
		const epic = { key: 'ABC-7', fact: fact('2026-11-18') };
		expect(epicNote(sp({ epic, on: '2026-11-18', holder: 'epic' }))).toBe('Epic ABC-7 is promised 18 Nov; the earlier date counts.');
		expect(epicNote(sp({ epic: { key: 'ABC-7', fact: fact('2026-11-30') }, holder: 'own' }))).toBeNull();
		expect(epicNote(null)).toBeNull();
	});
});

describe('promiseActions', () => {
	it.each<[boolean, boolean, string[]]>([
		[false, false, ['Promise by…']],
		[true, false, ['Change promise…', 'Clear promise']],
		[false, true, []],
		[true, true, ['Clear promise']]
	])('own %s, done %s', (own, done, labels) => expect(promiseActions(own, done)).toEqual(labels));
});

// Row for row the Rust table `matches_each_filter` in crates/reeve-planning/src/domain/promise.rs.
describe('promisedMatches', () => {
	const rows: [string, PromiseState | null, PromiseDue | null, [boolean, boolean, boolean, boolean]][] = [
		['open this sprint', 'open', 'this_sprint', [true, false, false, true]],
		['open next sprint', 'open', 'next_sprint', [false, true, false, true]],
		['open later', 'open', 'later', [false, false, false, true]],
		['overdue', 'overdue', null, [true, false, true, true]],
		['kept', 'kept', null, [false, false, false, false]],
		['late', 'late', null, [false, false, false, false]],
		['dropped', 'dropped', null, [false, false, false, false]],
		['no state', null, null, [false, false, false, false]]
	];
	const filters: PromisedFilter[] = ['this_sprint', 'next_sprint', 'overdue', 'any'];
	it.each(rows.flatMap(([label, state, due, want]) => filters.map((f, i) => [label, f, state, due, want[i]] as const)))(
		'%s / %s',
		(_, f, state, due, want) => expect(promisedMatches(f, { state, due })).toBe(want)
	);
});
