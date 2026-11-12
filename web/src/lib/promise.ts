// The words, tone and icon of a promise (kb/ADR-0025). Every value comes from the server: state, due period and days are
// derived on read there, so nothing here reads a clock or computes a date. The CLI uses the same words: the rows of
// promise.fixture.json pin both.
import type { EpicPromise, PromiseDue, PromiseFact, PromiseShortcut, PromiseState, PromisedFilter, SprintPromise, StoryPromise } from '#lib/api/types.ts';
import { dateLabel, dayLabel, short } from './format.ts';

export type PromiseTone = 'warn' | 'ready' | 'muted' | 'neutral';
export type PromiseIcon = 'clock' | 'check' | 'calendar' | null;

/** A piece of promise words: text, or an epic key the host shows apart (linked in a line, kept on one line in a chip). */
export type PromisePart = { text: string } | { epic: string };

export type ChipLook = {
	/** The chip's words; an epic it is via comes last, as its own part. */
	parts: PromisePart[];
	tone: PromiseTone;
	icon: PromiseIcon;
};

/** " · this sprint", " · next sprint", or nothing (later, or no period). */
function period(due: PromiseDue | null | undefined): string {
	return due === 'this_sprint' ? ' · this sprint' : due === 'next_sprint' ? ' · next sprint' : '';
}

/** " 3 d", or "" when the server sent no count. */
const daysText = (days: number | null | undefined) => (days == null ? '' : ` ${days} d`);

/** States with words; `unknown` (a state added after this build) is shown neutrally, like open: "Promised by 20 Nov". */
type Settled = Exclude<PromiseState, 'open' | 'unknown'>;

/** The one table of state words, lower case; every surface formats from it (the CLI copies it). */
const STATE_WORDS: Record<Settled, (days: number | null | undefined) => string> = {
	overdue: (days) => `overdue${daysText(days)}`,
	kept: () => 'kept',
	late: (days) => `done${daysText(days)} late`,
	dropped: () => 'dropped'
};

/** The state's words, or null for open, an unknown state and no state. */
function stateWords(p: { state?: PromiseState | null; days?: number | null }): string | null {
	const st = p.state ?? null;
	return st == null || st === 'open' || st === 'unknown' ? null : STATE_WORDS[st](p.days);
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

/** A story chip's words without the epic: "Overdue 3 d · 20 Nov", "Promised by 20 Nov · this sprint", … */
function storyChipBase(p: { on: string; state?: PromiseState | null; due?: PromiseDue | null; days?: number | null }): string {
	const d = dayLabel(p.on);
	const w = stateWords(p);
	return w != null ? `${capital(w)} · ${d}` : `Promised by ${d}${p.state === 'open' ? period(p.due) : ''}`;
}

/** The tone and icon of a state; `past` is an epic's date gone by, which warns like an overdue story. The text always
 *  carries the state too. */
function look(state: PromiseState | 'past' | null | undefined): { tone: PromiseTone; icon: PromiseIcon } {
	if (state === 'overdue' || state === 'past') return { tone: 'warn', icon: 'clock' };
	if (state === 'kept') return { tone: 'ready', icon: 'check' };
	if (state === 'late' || state === 'dropped') return { tone: 'muted', icon: null };
	return { tone: 'neutral', icon: 'calendar' };
}

/** Whether a promise is an epic's own (`EpicPromiseDto`). */
const isEpic = (p: StoryPromise | EpicPromise | SprintPromise): p is EpicPromise => 'fact' in p;
/** Whether a promise is a row of a sprint's promise list (`SprintPromiseDto`). */
const isSprintRow = (p: StoryPromise | SprintPromise): p is SprintPromise => 'title' in p;

/** The words, tone and icon of a promise's chip: a story's or an epic's, or a sprint's promise row. A sprint row has no
 *  period: the row is about the sprint the date falls in. This is the only place a chip's words are built. */
export function chip(p: StoryPromise | EpicPromise | SprintPromise, { via = true }: ChipOptions = {}): ChipLook {
	if (isEpic(p)) {
		const d = dayLabel(p.fact.on);
		return { parts: [{ text: p.past ? `Past ${d}` : `Promised by ${d}${period(p.due)}` }], ...look(p.past ? 'past' : null) };
	}
	const row = isSprintRow(p);
	const base = storyChipBase(row ? { ...p, due: null } : p);
	const key = !via || p.holder !== 'epic' ? null : row ? (p.epic_key ?? null) : (p.epic?.key ?? null);
	return { parts: key == null ? [{ text: base }] : [{ text: base }, { text: ' · via ' }, { epic: key }], ...look(p.state) };
}

/** A row of a sprint's promise list after the story's title: the recipient, then the chip's words. */
export function sprintRowParts(p: SprintPromise): PromisePart[] {
	const parts = chip(p).parts;
	return p.to ? [{ text: `to ${p.to} · ` }, ...parts] : parts;
}

const to = (f: PromiseFact) => (f.to ? ` to ${f.to}` : '');
const setBy = (f: PromiseFact) => ` · set by ${short(f.by)}`;

/** The promise line on the story drawer and the epic page, in parts so the host can link the epic's key. A line gives
 *  the full date, year included: a kept promise stays on a done story for good. */
export function lineParts(p: StoryPromise | EpicPromise): PromisePart[] {
	if (isEpic(p)) return [{ text: `Promised by ${dateLabel(p.fact.on)}${to(p.fact)}${setBy(p.fact)}${p.past ? ' · past' : ''}` }];
	const own = p.own ?? null;
	const epic = p.epic ?? null;
	let parts: PromisePart[];
	const w = stateWords(p);
	let suffix = stateSuffix(w);
	if (own && epic) {
		suffix = stateSentence(w);
		parts = [
			{ text: `Promised by ${dateLabel(own.on)}${to(own)}. Epic ` },
			{ epic: epic.key },
			{ text: ` is promised by ${dateLabel(epic.fact.on)}${to(epic.fact)}${p.holder === 'epic' ? '; the earlier date counts.' : '.'}` }
		];
	} else if (epic) {
		parts = [
			{ text: `Promised by ${dateLabel(epic.fact.on)} via epic ` },
			{ epic: epic.key },
			{ text: `${to(epic.fact)}${setBy(epic.fact)}` }
		];
	} else if (own) {
		parts = [{ text: `Promised by ${dateLabel(own.on)}${to(own)}${setBy(own)}` }];
	} else {
		parts = [{ text: `Promised by ${dateLabel(p.on)}` }];
	}
	return suffix ? [...parts, { text: suffix }] : parts;
}

/** The epic promise a story inherits, as the promise dialog takes it, or null. */
export function inheritedEpic(p: StoryPromise | null | undefined): { key: string; on: string } | null {
	const epic = p?.epic ?? null;
	return epic ? { key: epic.key, on: epic.fact.on } : null;
}

/** The dialog's note about the inherited epic promise, against the date chosen so far. ISO dates compare as strings; the
 *  story's own date wins a tie (R3). A UX affordance: the server derives which date counts on every read. */
export function epicNote(epic: { key: string; on: string }, chosen: string): string {
	const d = dateLabel(epic.on);
	return chosen !== '' && epic.on < chosen
		? `Epic ${epic.key} is promised by ${d}; the earlier date counts.`
		: `Epic ${epic.key} is promised by ${d}.`;
}

/** What a promise item of an actions menu does: open the dialog (to promise or change), or clear the own promise. */
export type PromiseActionKind = 'set' | 'clear';
export type PromiseAction = { label: string; kind: PromiseActionKind };

/** The promise items of an actions menu. A done story cannot be promised (R2), but its own promise can be cleared; an
 *  epic passes `done = false`. */
export function promiseActions(hasOwn: boolean, done: boolean): PromiseAction[] {
	const clear: PromiseAction = { label: 'Clear promise', kind: 'clear' };
	if (hasOwn) return done ? [clear] : [{ label: 'Change promise…', kind: 'set' }, clear];
	return done ? [] : [{ label: 'Promise by…', kind: 'set' }];
}

/** The story list's `promised` filter over the server's `state` and `due`, as the server applies it (the Rust
 *  `Filter::matches`; the fixture's `filter` rows pin both): this sprint keeps overdue and open-this-sprint, next sprint
 *  open-next-sprint, overdue only overdue, any every open or overdue promise. */
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

/** The Promise filter's labels: a Record, so a value the server adds fails `npm run check` until it is named here. */
const PROMISED_LABELS: Record<PromisedFilter, string> = {
	any: 'Promised (open or overdue)',
	this_sprint: 'Due this sprint, incl. overdue',
	next_sprint: 'Due next sprint',
	overdue: 'Overdue'
};

/** The Promise select's empty option (no filter). The other selects say "any"; here that word is one of the values. */
export const PROMISED_NONE = 'all stories';

/** The Promise filter's options on the board and the backlog, after the empty `PROMISED_NONE`, in the labels' order. */
export const PROMISED_OPTIONS: readonly { value: PromisedFilter; label: string }[] = (Object.keys(PROMISED_LABELS) as PromisedFilter[]).map(
	(value) => ({ value, label: PROMISED_LABELS[value] })
);

/** Whether a URL value is one of the `promised` filter's values (as `isKind` is for kinds). */
export function isPromisedFilter(s: string | null | undefined): s is PromisedFilter {
	return PROMISED_OPTIONS.some((o) => o.value === s);
}

/** A sprint-end shortcut: which end, its words with the day, and the date it fills. */
export type SprintEnd = { s: PromiseShortcut; label: string; ends: string };

/** The sprint-end shortcuts on `today` (the server's date): the active sprint's end and the draft's, the next sprint
 *  (kb/ADR-0025, kb/ADR-0026), each only when it is today or later (a promise refuses a past date; a past end is no use as a
 *  due date either). None while `today` is unknown. The promise dialog and a decision's Due date offer the same ones. */
export function sprintEnds(activeEnds: string | null, draftEnds: string | null, today: string | null): SprintEnd[] {
	if (today == null) return [];
	const ends = [
		['this_sprint', 'End of this sprint', activeEnds],
		['next_sprint', 'End of next sprint', draftEnds]
	] as const;
	return ends.flatMap(([s, label, on]) => (on != null && on >= today ? [{ s, label: `${label} (${dayLabel(on)})`, ends: on }] : []));
}
