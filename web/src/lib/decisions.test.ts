import { describe, expect, it } from 'vitest';
import type { RequiredDecision } from '#lib/api/types.ts';
import {
	LANE_LABEL,
	LANE_ORDER,
	blocking,
	checkLabel,
	closeActions,
	dateLine,
	pruneExpanded,
	decisionStatus,
	filterDecisions,
	itemHref,
	neededInText,
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
		expect(nextStepText({ kind: 'fix_record', result: { kind: 'not_accepted', status: 'superseded', superseded_by: 'kb/ADR-9', commit: 'abc' } }, null)).toBe('superseded by kb/ADR-9, re-point');
		expect(nextStepText({ kind: 'review_record', pr_url: 'https://github.com/x/y/pull/1' }, null)).toBe('Linked PR \u00b7 not accepted yet');
		expect(nextStepText({ kind: 'await_check' }, null)).toBe('Waiting for the check (never ran)');
		const now = Date.parse('2026-11-10T12:00:00Z');
		expect(nextStepText({ kind: 'await_check', checked_at: '2026-11-10T09:00:00Z' }, null, now)).toBe('Waiting for the check (last ran 3h ago)');
	});

	it('says why a decision is needed and in which sprint', () => {
		expect(neededInText({ kind: 'goal', goal_id: 1, outcome: 'Ship X' })).toBe('for goal: Ship X');
		expect(neededInText({ kind: 'discovery', key: 'GRA-12', level: 'initiative' })).toBe('GRA-12 is in discovery');
		expect(neededLabel('this_sprint')).toBe('Needed this sprint');
		expect(neededLabel('next_sprint')).toBe('Needed next sprint');
		expect(neededLabel('beyond')).toBe('Needed beyond');
		expect(neededLabel('none')).toBe('Not needed yet');
	});

	it('orders and labels every lane', () => {
		expect(LANE_ORDER).toEqual(['review', 'this_sprint', 'next_sprint', 'beyond', 'open', 'satisfied', 'closed']);
		for (const l of LANE_ORDER) expect(LANE_LABEL[l]).toBeTruthy();
	});
});
