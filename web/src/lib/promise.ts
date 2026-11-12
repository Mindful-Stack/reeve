// The words, tone and icon of a promise (kb/ADR-0025). Every value comes from the server: state, due period and days are
// derived on read there, so nothing here reads a clock or computes a date. The CLI uses the same words.
import type { EpicPromise, PromiseDue, PromiseFact, PromiseState, PromisedFilter, StoryPromise } from '#lib/api/types.ts';
import { dayLabel, short } from './format.ts';

export type PromiseTone = 'warn' | 'ready' | 'muted' | 'neutral';
export type PromiseIcon = 'clock' | 'check' | 'calendar' | null;
export type ChipLook = { text: string; tone: PromiseTone; icon: PromiseIcon };

/** " · this sprint", " · next sprint", or nothing (later, or no period). */
function period(due: PromiseDue | null | undefined): string {
	return due === 'this_sprint' ? ' · this sprint' : due === 'next_sprint' ? ' · next sprint' : '';
}

/** "3 d", or "" when the server sent no count. */
const daysText = (days: number | null | undefined) => (days == null ? '' : ` ${days} d`);

export type ChipOptions = {
	/** Name the epic (" · via ABC-7") when its promise sets the date. False on the epic's own page, where every row is
	 *  under that epic. Default true. */
	via?: boolean;
};

/** A story chip's text, from the server's effective date, state, period and days. */
export function chipText(
	p: Pick<StoryPromise, 'on' | 'state' | 'due' | 'days' | 'holder' | 'epic'>,
	{ via = true }: ChipOptions = {}
): string {
	const d = dayLabel(p.on);
	const state = p.state ?? null;
	const text =
		state === 'overdue'
			? `Overdue${daysText(p.days)} · ${d}`
			: state === 'kept'
				? `Kept · ${d}`
				: state === 'late'
					? `Done${daysText(p.days)} late · ${d}`
					: state === 'dropped'
						? `Dropped · ${d}`
						: `Promised ${d}${state === 'open' ? period(p.due) : ''}`;
	return via && p.holder === 'epic' && p.epic != null ? `${text} · via ${p.epic.key}` : text;
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

/** Everything a chip shows, for a story's or an epic's promise. */
export function chip(p: StoryPromise | EpicPromise, opts: ChipOptions = {}): ChipLook {
	return isEpic(p)
		? { text: epicChipText(p), ...look(p.past ? 'past' : null) }
		: { text: chipText(p, opts), ...look(p.state) };
}

const to = (f: PromiseFact) => (f.to ? ` to ${f.to}` : '');
const setBy = (f: PromiseFact) => ` · set by ${short(f.by)}`;

/** The state as its own sentence, after a line that names two promises: " Overdue 3 d.", " Kept.", … */
function stateSentence(p: Pick<StoryPromise, 'state' | 'days'>): string {
	switch (p.state ?? null) {
		case 'overdue':
			return ` Overdue${daysText(p.days)}.`;
		case 'kept':
			return ' Kept.';
		case 'late':
			return ` Done${daysText(p.days)} late.`;
		case 'dropped':
			return ' Dropped.';
		default:
			return '';
	}
}

function stateSuffix(p: Pick<StoryPromise, 'state' | 'days'>): string {
	switch (p.state ?? null) {
		case 'overdue':
			return ` · overdue${daysText(p.days)}`;
		case 'kept':
			return ' · kept';
		case 'late':
			return ` · done${daysText(p.days)} late`;
		case 'dropped':
			return ' · dropped';
		default:
			return '';
	}
}

/** A piece of a promise line: text, or an epic key the line links. */
export type LinePart = { text: string } | { epic: string };

/** The drawer's promise line, in parts so the host can link the epic's key. */
export function lineParts(p: StoryPromise): LinePart[] {
	const own = p.own ?? null;
	const epic = p.epic ?? null;
	let parts: LinePart[];
	let suffix = stateSuffix(p);
	if (own && epic) {
		suffix = stateSentence(p);
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
