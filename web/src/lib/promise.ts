// The words, tone and icon of a promise (kb/ADR-0025). Every value comes from the server: state, due period and days are
// derived on read there, so nothing here reads a clock or computes a date. The CLI uses the same words.
import type { EpicPromise, PromiseDue, PromiseFact, PromiseState, PromisedFilter, StoryPromise } from '#lib/api/types.ts';
import { dayLabel, short } from './format.ts';

export type PromiseTone = 'warn' | 'ready' | 'muted' | 'neutral';
export type PromiseIcon = 'clock' | 'check' | 'calendar' | null;
export type ChipLook = {
	/** The chip's words, without the epic it is via. */
	text: string;
	/** The epic key the chip names (" · via ABC-7"), or null. */
	via: string | null;
	tone: PromiseTone;
	icon: PromiseIcon;
};

/** " · this sprint", " · next sprint", or nothing (later, or no period). */
function period(due: PromiseDue | null | undefined): string {
	return due === 'this_sprint' ? ' · this sprint' : due === 'next_sprint' ? ' · next sprint' : '';
}

/** " 3 d", or "" when the server sent no count. */
const daysText = (days: number | null | undefined) => (days == null ? '' : ` ${days} d`);

type Settled = Exclude<PromiseState, 'open'>;

/** The one table of state words, lower case; every surface formats from it (the CLI copies it). */
export const STATE_WORDS: Record<Settled, (days: number | null | undefined) => string> = {
	overdue: (days) => `overdue${daysText(days)}`,
	kept: () => 'kept',
	late: (days) => `done${daysText(days)} late`,
	dropped: () => 'dropped'
};

/** The state's words, or null for open and no state. */
function stateWords(p: Pick<StoryPromise, 'state' | 'days'>): string | null {
	const st = p.state ?? null;
	return st == null || st === 'open' ? null : STATE_WORDS[st](p.days);
}

const capital = (w: string) => w.charAt(0).toUpperCase() + w.slice(1);

/** The state at the end of a one-promise line: " · overdue 3 d", " · kept", … */
const stateSuffix = (w: string | null) => (w == null ? '' : ` · ${w}`);
/** The state as its own sentence, after a line that names two promises: " Overdue 3 d.", " Kept.", … */
const stateSentence = (w: string | null) => (w == null ? '' : ` ${capital(w)}.`);

export type ChipOptions = {
	/** Name the epic (" · via ABC-7") when its promise sets the date. False on the epic's own page, where every row is
	 *  under that epic. Default true. */
	via?: boolean;
};

/** A story chip's words without the epic: "Overdue 3 d · 20 Nov", "Promised 20 Nov · this sprint", … */
function storyChipBase(p: Pick<StoryPromise, 'on' | 'state' | 'due' | 'days'>): string {
	const d = dayLabel(p.on);
	const w = stateWords(p);
	return w != null ? `${capital(w)} · ${d}` : `Promised ${d}${p.state === 'open' ? period(p.due) : ''}`;
}

/** The epic key a story chip names, or null. */
function viaKey(p: Pick<StoryPromise, 'holder' | 'epic'>, { via = true }: ChipOptions): string | null {
	return via && p.holder === 'epic' && p.epic != null ? p.epic.key : null;
}

/** A story chip's text, from the server's effective date, state, period and days. */
export function chipText(p: Pick<StoryPromise, 'on' | 'state' | 'due' | 'days' | 'holder' | 'epic'>, opts: ChipOptions = {}): string {
	const key = viaKey(p, opts);
	return `${storyChipBase(p)}${key != null ? ` · via ${key}` : ''}`;
}

/** An epic chip's text: an epic has no state of its own (R4), only a period while the date is not past. */
export function epicChipText(p: EpicPromise): string {
	const d = dayLabel(p.fact.on);
	return p.past ? `Past ${d}` : `Promised ${d}${period(p.due)}`;
}

/** The tone and icon of a state; `past` is an epic's date gone by. The text always carries the state too. */
export function look(state: PromiseState | 'past' | null | undefined): { tone: PromiseTone; icon: PromiseIcon } {
	if (state === 'overdue') return { tone: 'warn', icon: 'clock' };
	if (state === 'kept') return { tone: 'ready', icon: 'check' };
	if (state === 'late' || state === 'dropped' || state === 'past') return { tone: 'muted', icon: null };
	return { tone: 'neutral', icon: 'calendar' };
}

/** Whether a promise is an epic's own (`EpicPromiseDto`) rather than a story's. */
export const isEpic = (p: StoryPromise | EpicPromise): p is EpicPromise => 'fact' in p;

/** Everything a chip shows, for a story's or an epic's promise: the words and, apart, the epic key it is via. */
export function chip(p: StoryPromise | EpicPromise, opts: ChipOptions = {}): ChipLook {
	return isEpic(p)
		? { text: epicChipText(p), via: null, ...look(p.past ? 'past' : null) }
		: { text: storyChipBase(p), via: viaKey(p, opts), ...look(p.state) };
}

const to = (f: PromiseFact) => (f.to ? ` to ${f.to}` : '');
const setBy = (f: PromiseFact) => ` · set by ${short(f.by)}`;

/** A piece of a promise line: text, or an epic key the line links. */
export type LinePart = { text: string } | { epic: string };

/** The drawer's promise line, in parts so the host can link the epic's key. */
export function lineParts(p: StoryPromise): LinePart[] {
	const own = p.own ?? null;
	const epic = p.epic ?? null;
	let parts: LinePart[];
	const w = stateWords(p);
	let suffix = stateSuffix(w);
	if (own && epic) {
		suffix = stateSentence(w);
		parts = [
			{ text: `Promised ${dayLabel(own.on)}${to(own)}. Epic ` },
			{ epic: epic.key },
			{ text: ` is promised ${dayLabel(epic.fact.on)}${p.holder === 'epic' ? '; the earlier date counts.' : '.'}` }
		];
	} else if (epic) {
		parts = [
			{ text: `Promised ${dayLabel(epic.fact.on)} via epic ` },
			{ epic: epic.key },
			{ text: `${to(epic.fact)}${setBy(epic.fact)}` }
		];
	} else if (own) {
		parts = [{ text: `Promised ${dayLabel(own.on)}${to(own)}${setBy(own)}` }];
	} else {
		parts = [{ text: `Promised ${dayLabel(p.on)}` }];
	}
	return suffix ? [...parts, { text: suffix }] : parts;
}

/** The drawer's promise line as plain text. */
export function lineText(p: StoryPromise): string {
	return lineParts(p)
		.map((x) => ('text' in x ? x.text : x.epic))
		.join('');
}

/** The epic page's promise line. */
export function epicLineText(p: EpicPromise): string {
	return `Promised ${dayLabel(p.fact.on)}${to(p.fact)}${setBy(p.fact)}${p.past ? ' · past' : ''}`;
}

/** The epic promise a story inherits, as the promise dialog takes it, or null. */
export function inheritedEpic(p: StoryPromise | null | undefined): { key: string; on: string } | null {
	const epic = p?.epic ?? null;
	return epic ? { key: epic.key, on: epic.fact.on } : null;
}

/** The dialog's note about the inherited epic promise, against the date chosen so far. ISO dates compare as strings; the
 *  story's own date wins a tie (R3). */
export function epicNote(epic: { key: string; on: string }, chosen: string): string {
	const d = dayLabel(epic.on);
	return chosen !== '' && epic.on < chosen
		? `Epic ${epic.key} is promised ${d}; the earlier date counts.`
		: `Epic ${epic.key} is promised ${d}.`;
}

export const PROMISE_BY = 'Promise by…';
export const CHANGE_PROMISE = 'Change promise…';
export const CLEAR_PROMISE = 'Clear promise';

/** The promise items of an actions menu. A done story cannot be promised (R2), but its own promise can be cleared; an
 *  epic passes `done = false`. */
export function promiseActions(hasOwn: boolean, done: boolean): string[] {
	if (hasOwn) return done ? [CLEAR_PROMISE] : [CHANGE_PROMISE, CLEAR_PROMISE];
	return done ? [] : [PROMISE_BY];
}

/** The story list's `promised` filter over the server's `state` and `due`, as the server applies it (the Rust
 *  `Filter::matches`): this sprint keeps overdue and open-this-sprint, next sprint open-next-sprint, overdue only
 *  overdue, any every open or overdue promise. */
export function promisedMatches(
	f: PromisedFilter,
	p: { state?: PromiseState | null; due?: PromiseDue | null }
): boolean {
	const state = p.state ?? null;
	switch (f) {
		case 'this_sprint':
			return state === 'overdue' || (state === 'open' && p.due === 'this_sprint');
		case 'next_sprint':
			return state === 'open' && p.due === 'next_sprint';
		case 'overdue':
			return state === 'overdue';
		case 'any':
			return state === 'open' || state === 'overdue';
	}
}
