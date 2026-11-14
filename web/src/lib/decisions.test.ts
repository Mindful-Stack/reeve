import { describe, expect, it } from 'vitest';
import type { DecisionQueueEntry, NeededInReason, QueueLane, RequiredDecision } from '#lib/api/types.ts';
import { blockedStory, discoveryEntry, queueEntry } from '#lib/test/fixtures.ts';
import {
	LANE_LABEL,
	LANE_ORDER,
	blocking,
	checkLabel,
	closeActions,
	dateLine,
	pruneExpanded,
	queueRows,
	reasonLine,
	type PageLane,
	type QueueRow,
	reasonHref,
	decisionStatus,
	filterDecisions,
	itemHref,
	holdsLabel,
	neededLabel,
	nextStepText,
	overdue,
	recordMismatch,
	sameEmail,
	today
} from '#lib/decisions.ts';

function dec(id: number, over: Partial<RequiredDecision> = {}): RequiredDecision {
	return {
		id,
		on: 'ABC-1',
		on_type: 'story',
		title: `decision ${id}`,
		decider: 'ana@x.se',
		due: null,
		record: null,
		pr_url: null,
		check: { kind: 'unchecked' },
		checked_at: null,
		closed: null,
		created_at: '2026-10-01T00:00:00Z',
		version: 1,
		...over
	};
}

describe('checkLabel', () => {
	it('tells a superseded record to re-point, naming its successor', () => {
		const text = checkLabel({
			kind: 'not_accepted',
			status: 'superseded',
			superseded_by: 'ADR-0016',
			commit: 'abc'
		});
		expect(text).toContain('re-point');
		expect(text).toContain('ADR-0016');
	});

	it('shows the short sha of an accepted record', () => {
		const text = checkLabel({ kind: 'accepted', commit: '0123456789abcdef' });
		expect(text).toContain('0123456');
		expect(text).not.toContain('01234567');
	});

	it('renders every other kind, and a not-accepted status as reported', () => {
		expect(checkLabel({ kind: 'unchecked' })).toBe('unchecked');
		expect(checkLabel({ kind: 'proposed_pr_open' })).toBe('proposed (PR linked)');
		expect(checkLabel({ kind: 'not_found' })).toBe('not found');
		expect(checkLabel({ kind: 'not_accepted', status: 'id mismatch', commit: 'abc' })).toBe('id mismatch');
	});

	it('says "no record" until a record is linked', () => {
		expect(decisionStatus(dec(1))).toBe('no record');
		const linked = dec(2, { record: { repo: 'kb', path: 'adrs/0001.md', record_id: 'kb/ADR-0001' } });
		expect(decisionStatus(linked)).toBe('unchecked');
	});
});

describe('filterDecisions', () => {
	const all = [
		dec(1),
		dec(2, { decider: ' Ana@X.se ' }),
		dec(3, { decider: 'bo@x.se' }),
		dec(4, { closed: { at: '2026-10-02T00:00:00Z', by: 'ana@x.se', reason: 'moot' } })
	];
	const ids = (f: Parameters<typeof filterDecisions>[1]) => filterDecisions(all, f).map((d) => d.id);

	it('hides closed decisions unless asked, and compares the decider case-insensitively', () => {
		expect(ids({})).toEqual([1, 2, 3]);
		expect(ids({ closed: true })).toEqual([1, 2, 3, 4]);
		expect(ids({ mine: 'ana@x.se' })).toEqual([1, 2]);
		expect(ids({ mine: 'ANA@x.se', closed: true })).toEqual([1, 2, 4]);
		expect(sameEmail('a@b.se', 'c@b.se')).toBe(false);
	});
});

describe('due dates and links', () => {
	it('is overdue only after the due day', () => {
		expect(overdue('2026-10-25', '2026-10-26')).toBe(true);
		expect(overdue('2026-10-26', '2026-10-26')).toBe(false);
		expect(overdue(null, '2026-10-26')).toBe(false);
		expect(today(new Date(2026, 0, 5))).toBe('2026-01-05');
	});

	it('links an initiative, an epic, and anything else to the story drawer', () => {
		const ini = new Set(['ABC-1']);
		const epics = new Set(['ABC-2']);
		expect(itemHref('ABC-1', ini, epics)).toBe('/initiatives/ABC-1');
		expect(itemHref('ABC-2', ini, epics)).toBe('/epics/ABC-2');
		expect(itemHref('ABC-3', ini, epics)).toBe('/board?story=ABC-3');
	});

	it('prunes expanded ids whose decision left the list', () => {
		const expanded = new Set([1, 2, 3]);
		pruneExpanded(expanded, [{ id: 1 }, { id: 3 }]);
		expect([...expanded]).toEqual([1, 3]);
	});
});

describe('blocking', () => {
	const closed = { by: 'x', at: '2026-10-02T00:00:00Z', reason: 'z' };
	it('an open unsatisfied decision blocks; accepted (satisfied) and closed ones do not', () => {
		const list = [
			dec(1, { check: { kind: 'unchecked' } }),
			dec(2, { check: { kind: 'accepted', commit: 'abc1234def' } }),
			dec(3, { check: { kind: 'proposed_pr_open' }, closed }),
			dec(4, { check: { kind: 'not_accepted', commit: 'abc1234def', status: 'rejected' } }),
			dec(5, { check: { kind: 'not_found' } }),
			dec(6, { check: { kind: 'proposed_pr_open' } })
		];
		expect(blocking(list).map((x) => x.id)).toEqual([1, 4, 5, 6]);
	});
});

describe('closeActions', () => {
	// R3 in the UI: one-way offers only withdraw. Catches unset treated as one-way.
	it('follows reversibility', () => {
		expect(closeActions('one_way')).toEqual(['withdrawn']);
		expect(closeActions('two_way')).toEqual(['decided', 'withdrawn']);
		expect(closeActions(null)).toEqual(['decided', 'withdrawn']);
	});
});

describe('dateLine', () => {
	// Catches off-by-one at today and the age shown when a date exists.
	it('words every case from the viewer date', () => {
		const on = '2026-11-03';
		expect(dateLine({ due: '2026-11-06', created_at: '2026-10-22T10:00:00Z' }, on)).toEqual({ text: 'Due in 3 days (Fri 6 Nov)', tone: 'normal' });
		expect(dateLine({ due: '2026-11-04', created_at: '2026-10-22T10:00:00Z' }, on)).toEqual({ text: 'Due in 1 day (Wed 4 Nov)', tone: 'normal' });
		expect(dateLine({ due: '2026-11-03', created_at: '2026-10-22T10:00:00Z' }, on)).toEqual({ text: 'Due today', tone: 'warning' });
		expect(dateLine({ due: '2026-11-01', created_at: '2026-10-22T10:00:00Z' }, on)).toEqual({ text: 'Overdue 2 days (Sun 1 Nov)', tone: 'danger' });
		expect(dateLine({ due: null, created_at: '2026-10-22T10:00:00Z' }, on)).toEqual({ text: 'No date · open 12 days', tone: 'warning' });
	});
});

describe('dateLine age', () => {
	// The age counts from the timestamp's local date, as `on` is local. Built from local Dates, so near midnight the UTC
	// date differs from the local one in any zone but UTC (red there; in a UTC runner both paths agree and it is a pin).
	it('counts open days from the local date of created_at', () => {
		for (const [h, m] of [[0, 30], [23, 30]]) {
			const created_at = new Date(2026, 9, 22, h, m).toISOString();
			expect(dateLine({ due: null, created_at }, '2026-11-03').text).toBe('No date · open 12 days');
		}
	});

	it('shows ? for an unparseable created_at', () => {
		expect(dateLine({ due: null, created_at: 'nope' }, '2026-11-03')).toEqual({ text: 'No date · open ?', tone: 'warning' });
	});
});

describe('recordMismatch', () => {
	// R2: only when both set and differ.
	it('warns only on a real disagreement', () => {
		const acc = (r?: 'one_way' | 'two_way') => ({ kind: 'accepted', commit: 'a', ...(r ? { reversibility: r } : {}) }) as const;
		expect(recordMismatch({ reversibility: 'one_way', check: acc('two_way') })).toBe('two_way');
		expect(recordMismatch({ reversibility: 'one_way', check: acc('one_way') })).toBeNull();
		expect(recordMismatch({ reversibility: null, check: acc('two_way') })).toBeNull();
		expect(recordMismatch({ reversibility: 'one_way', check: acc() })).toBeNull();
	});

	// The server never reports a foreign record's value, so an id mismatch carries none and is never a mismatch.
	it('never warns on an id mismatch', () => {
		const check = { kind: 'not_accepted', status: 'id mismatch', commit: 'a' } as const;
		expect(recordMismatch({ reversibility: 'one_way', check })).toBeNull();
	});
});

describe('next step and needed-in wording', () => {
	it('says what each next step needs', () => {
		expect(nextStepText({ kind: 'write_record' }, null)).toBe('Write a record, or close it');
		expect(nextStepText({ kind: 'write_record' }, 'one_way')).toBe('Write a record');
		expect(nextStepText({ kind: 'configure_repo', repo: 'infra' }, null)).toBe("Repository infra is not checked: an admin adds it to the server's repos");
		expect(nextStepText({ kind: 'fix_record', result: { kind: 'not_accepted', status: 'superseded', superseded_by: 'kb/ADR-9', commit: 'abc' } }, null)).toBe('Fix the record: superseded by kb/ADR-9, re-point');
		expect(nextStepText({ kind: 'fix_record', result: { kind: 'not_found' } }, null)).toBe('Fix the record: not found');
		expect(nextStepText({ kind: 'review_record', pr_url: 'https://github.com/x/y/pull/1' }, null)).toBe('Linked PR \u00b7 not accepted yet');
		expect(nextStepText({ kind: 'await_check' }, null)).toBe('Waiting for the check (never ran)');
		const now = Date.parse('2026-11-10T12:00:00Z');
		expect(nextStepText({ kind: 'await_check', checked_at: '2026-11-10T09:00:00Z' }, null, now)).toBe('Waiting for the check (last ran 3h ago)');
	});

	it('reads every reason kind off a decision row and a discovery row: its line and where it links', () => {
		const board = new URL('https://reeve.test/board');
		const due = queueEntry({ decision: dec(4, { due: '2026-11-17' }), blocks: [blockedStory()], under_way: [] });
		const dRow: QueueRow = { kind: 'decision', entry: due };
		const nested: QueueRow = { kind: 'discovery', row: discoveryEntry({ key: 'ABC-2', blocks: [], under_way: [blockedStory(), blockedStory()] }), nested: [due] };
		const line = (r: NeededInReason, row: QueueRow) => {
			const p = reasonLine(r, row);
			return [p.before + (p.key && `<${p.key}>`) + p.after, reasonHref(r, board)];
		};
		const goal = { kind: 'goal', goal_id: 1, outcome: 'Ship X' } as const;
		const promise = { kind: 'promise', key: 'ABC-12', level: 'story', on: '2026-11-20', overdue: false } as const;
		// A kind newer than the generated types still reads safely at run time.
		const future = { kind: 'future_thing' } as never;
		const table: [string, NeededInReason, QueueRow, string, string | null][] = [
			['goal', goal, dRow, 'for goal: Ship X', null],
			['discovery', { kind: 'discovery', key: 'GRA-12', level: 'initiative' }, dRow, '<GRA-12> is in discovery', '/initiatives/GRA-12'],
			['discovery epic', { kind: 'discovery', key: 'GRA-13', level: 'epic' }, dRow, '<GRA-13> is in discovery', '/epics/GRA-13'],
			['promise story', promise, dRow, 'promised by 20 Nov: <ABC-12>', '/board?story=ABC-12'],
			['promise overdue epic', { ...promise, key: 'ABC-5', level: 'epic', on: '2026-11-10', overdue: true }, dRow, 'promised by 10 Nov, overdue: <ABC-5>', '/epics/ABC-5'],
			['urgent epic', { kind: 'urgent', key: 'ABC-5', level: 'epic' }, dRow, 'Now · urgent via <ABC-5>', '/epics/ABC-5'],
			['urgent story', { kind: 'urgent', key: 'ABC-6', level: 'story' }, nested, 'Now · urgent via <ABC-6>', '/board?story=ABC-6'],
			['target month', { kind: 'target_month', month: '2026-11' }, dRow, 'Now · target Nov 2026', null],
			['due, a decision', { kind: 'decision_due', id: 4 }, dRow, 'Now · due 17 Nov', '/board?decision=4'],
			['due, a row', { kind: 'decision_due', id: 4 }, nested, 'Now · <decision #4> due 17 Nov', '/board?decision=4'],
			['held, a decision', { kind: 'held' }, dRow, 'Now · holds 1', null],
			['held, a row holding none', { kind: 'held' }, nested, 'Now · holds none, 2 under way', null],
			['unknown', { kind: 'unknown' }, dRow, 'needed (reason not shown)', null],
			['a future kind', future, dRow, 'needed (reason not shown)', null]
		];
		expect(table.map(([name, r, row]) => [name, ...line(r, row)])).toEqual(table.map(([name, , , text, href]) => [name, text, href]));
	});

	it('labels when a decision is needed by its lane, and a nested one as in discovery', () => {
		const e = (more: Partial<DecisionQueueEntry>) => queueEntry(more);
		expect(neededLabel(e({ lane: 'this_sprint', needed_in: 'beyond' }))).toBe('Needed this sprint');
		expect(neededLabel(e({ lane: 'next_sprint' }))).toBe('Needed next sprint');
		expect(neededLabel(e({ lane: 'beyond' }))).toBe('Needed later');
		// Now says so in its reason; open, accepted and closed decisions are needed nowhere.
		expect(neededLabel(e({ lane: 'beyond', needed_in_reason: { kind: 'held' } }))).toBeNull();
		for (const lane of ['open', 'satisfied', 'closed'] as const) expect(neededLabel(e({ lane }))).toBeNull();
		// Nested: the row's lane, "in discovery" even with no window of its own; under a Now row it is not Later.
		expect(neededLabel(e({ lane: 'next_sprint', needed_in: 'none', under: 'ABC-2' }))).toBe('Needed next sprint \u00b7 in discovery');
		expect(neededLabel(e({ lane: 'beyond', under: 'ABC-2' }))).toBe('in discovery');
	});

	it('says what a row holds, and only the stories under way when it holds none', () => {
		const some = [blockedStory({ key: 'ABC-1' }), blockedStory({ key: 'ABC-2' })];
		expect(holdsLabel(some, [])).toBe('Holds 2 stories');
		expect(holdsLabel(some.slice(0, 1), some)).toBe('Holds 1 story \u00b7 2 under way');
		expect(holdsLabel([], some)).toBe('2 under way');
	});

	it('orders and labels every lane', () => {
		expect(LANE_ORDER).toEqual(['this_sprint', 'next_sprint', 'now', 'later', 'open', 'satisfied', 'closed']);
		for (const l of LANE_ORDER) expect(LANE_LABEL[l]).toBeTruthy();
	});
});

describe('queueRows', () => {
	const e = (id: number, lane: QueueLane, more: Partial<DecisionQueueEntry> = {}) => queueEntry({ lane, decision: dec(id), ...more });
	const bob = (id: number, more: Partial<RequiredDecision> = {}) => dec(id, { decider: 'bob@x.se', ...more });
	/** Each non-empty lane's rows: a decision by its id, a discovery row as KEY[nested ids]. */
	const shape = (m: Map<PageLane, QueueRow[]>) =>
		Object.fromEntries(
			[...m]
				.filter(([, rows]) => rows.length > 0)
				.map(([l, rows]) => [
					l,
					rows.map((r) => (r.kind === 'decision' ? r.entry.decision.id : `${r.row.key}[${r.nested.map((n) => n.decision.id).join(',')}]`))
				])
		);
	// Server page order: this sprint (row ABC-2 with D1, then D2); Now (D3, row ABC-5 with bob's D4 and ana's D8, its
	// reason D4's due date, then row ABC-7 with bob's D9); Later; Open; Closed.
	const q = {
		entries: [
			e(1, 'this_sprint', { under: 'ABC-2' }),
			e(2, 'this_sprint'),
			e(3, 'beyond', { needed_in_reason: { kind: 'held' }, decision: bob(3) }),
			e(4, 'beyond', { under: 'ABC-5', decision: bob(4, { due: '2026-11-17' }) }),
			e(8, 'beyond', { under: 'ABC-5' }),
			e(9, 'beyond', { under: 'ABC-7', decision: bob(9) }),
			e(5, 'beyond'),
			e(6, 'open', { under: 'ABC-9' }),
			e(7, 'closed')
		],
		discoveries: [
			discoveryEntry({ key: 'ABC-2', lane: 'this_sprint', at: 0 }),
			discoveryEntry({ key: 'ABC-5', lane: 'beyond', needed_in_reason: { kind: 'decision_due', id: 4 }, at: 3 }),
			discoveryEntry({ key: 'ABC-7', lane: 'beyond', needed_in_reason: { kind: 'held' }, at: 5 })
		]
	};

	it('puts each row before entries[at], nests by `under`, and splits beyond into Now and Later', () => {
		expect(shape(queueRows(q))).toEqual({
			this_sprint: ['ABC-2[1]', 2],
			now: [3, 'ABC-5[4,8]', 'ABC-7[9]'],
			later: [5],
			// `under` naming a row the response does not carry: the decision stands alone.
			open: [6],
			closed: [7]
		});
	});

	it("reads an older server's review lane as open", () => {
		expect(shape(queueRows({ entries: [e(1, 'review')], discoveries: [] }))).toEqual({ open: [1] });
	});

	it('puts a row whose `at` is the end after every entry', () => {
		expect(shape(queueRows({ entries: [e(2, 'this_sprint')], discoveries: [discoveryEntry({ key: 'ABC-3', lane: 'next_sprint', at: 1 })] }))).toEqual({
			this_sprint: [2],
			next_sprint: ['ABC-3[]']
		});
	});

	it('Mine keeps my standalone decisions and a row that nests one of mine, whole, so its reason can name another decider', () => {
		const mine = queueRows(q, { me: ' ANA@x.se ' });
		expect(shape(mine)).toEqual({
			this_sprint: ['ABC-2[1]', 2],
			now: ['ABC-5[4,8]'],
			later: [5],
			open: [6],
			closed: [7]
		});
		// The kept row's reason names bob's D4, and its date is found although D4 is not mine.
		const row = mine.get('now')![0];
		expect(reasonLine({ kind: 'decision_due', id: 4 }, row)).toEqual({ before: 'Now · ', key: 'decision #4', after: ' due 17 Nov' });
	});
});
