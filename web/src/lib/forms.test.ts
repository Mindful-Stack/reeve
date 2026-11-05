import { describe, expect, it } from 'vitest';
import {
	DECISION_SAME, EPIC_SAME, INITIATIVE_SAME, decisionEdit, epicCreate, epicDetails, epicEdit, fieldsText,
	decisionDraft, initiativeCreate, initiativeDetails, initiativeEdit, mergeDraft, validMonth
} from './forms.ts';
import type { DecisionDraft } from './forms.ts';
import { decision, epic, initiative } from '#lib/test/fixtures.ts';

describe('validMonth', () => {
	it.each([
		['2026-11', true], ['2026-01', true], ['2026-12', true], ['2026-00', false], ['2026-13', false],
		['2026-1', false], ['26-11', false], ['2026-11-01', false], [' 2026-11', false], ['', false]
	])('%j → %s', (s, ok) => expect(validMonth(s)).toBe(ok));
});

describe('initiativeCreate', () => {
	const buckets = ['Now', 'Next'];
	const draft = { title: '  Orion  ', horizon: 'Now', target_month: '', brief: '' };
	it('trims the title and sends an empty month and brief as null', () => {
		const p = initiativeCreate(draft, buckets);
		expect(p.ok && p.body).toEqual({ title: 'Orion', horizon: 'Now', target_month: null, brief: null, owner: null });
	});
	it.each([
		[{ title: '   ' }, 'Title must not be blank.'],
		[{ horizon: 'Someday' }, 'Choose a horizon.'],
		[{ target_month: '2026-13' }, 'Target month must be YYYY-MM.']
	])('refuses %j', (patch, error) => expect(initiativeCreate({ ...draft, ...patch }, buckets)).toEqual({ ok: false, error }));
	it('sends a whitespace-only brief as null', () => {
		const p = initiativeCreate({ ...draft, brief: '  \n' }, buckets);
		expect(p).toMatchObject({ ok: true, body: { brief: null } });
	});
	it('gives equal content one text and any changed field another', () => {
		const text = (d: typeof draft) => {
			const p = initiativeCreate(d, buckets);
			if (!p.ok) throw new Error(p.error);
			return p.text;
		};
		expect(text({ ...draft, title: 'Orion' })).toBe(text(draft));
		for (const patch of [{ title: 'Orion 2' }, { horizon: 'Next' }, { target_month: '2026-11' }, { brief: 'why' }]) {
			expect(text({ ...draft, ...patch })).not.toBe(text(draft));
		}
	});
});

describe('epicCreate', () => {
	it('sends no initiative as null and a chosen one by key', () => {
		const none = epicCreate({ title: ' E ', brief: '', initiative: '' });
		expect(none.ok && none.body).toEqual({ title: 'E', brief: null, initiative: null });
		const under = epicCreate({ title: 'E', brief: '', initiative: 'ABC-1' });
		expect(under.ok && under.body.initiative).toBe('ABC-1');
		expect(under.ok && none.ok && under.text !== none.text).toBe(true);
	});
	it('sends a whitespace-only brief as null', () => {
		expect(epicCreate({ title: 'E', brief: '  \n', initiative: '' })).toMatchObject({ ok: true, body: { brief: null } });
	});
	it('refuses a blank title', () => {
		expect(epicCreate({ title: ' ', brief: '', initiative: '' })).toEqual({ ok: false, error: 'Title must not be blank.' });
	});
});

describe('initiativeEdit', () => {
	const from = { title: 'Orion', target_month: '2026-11', owner: 'dan@example.com' };
	it.each([
		['nothing changed', {}, { kind: 'unchanged' }],
		['the title only re-spaced', { title: ' Orion ' }, { kind: 'unchanged' }],
		['the owner only re-cased', { owner: 'Dan@Example.com' }, { kind: 'unchanged' }],
		['the month only re-spaced', { target_month: ' 2026-11 ' }, { kind: 'unchanged' }],
		['the title changed and re-spaced', { title: ' Orion 2 ' }, { kind: 'send', body: { title: 'Orion 2' } }],
		['a month re-spaced and changed', { target_month: ' 2026-12 ' }, { kind: 'send', body: { target_month: '2026-12' } }],
		['the title', { title: 'Orion 2' }, { kind: 'send', body: { title: 'Orion 2' } }],
		['the month cleared', { target_month: '' }, { kind: 'send', body: { target_month: null } }],
		['the owner cleared', { owner: '  ' }, { kind: 'send', body: { owner: null } }],
		['another owner', { owner: 'bob@example.com' }, { kind: 'send', body: { owner: 'bob@example.com' } }],
		['a blank title', { title: ' ' }, { kind: 'invalid', error: 'Title must not be blank.' }],
		['a bad month', { target_month: '2026-13' }, { kind: 'invalid', error: 'Target month must be YYYY-MM.' }]
	])('%s', (_, patch, want) => expect(initiativeEdit(from, { ...from, ...patch })).toEqual(want));
	it('never blocks a save on a stored month the user did not touch', () => {
		const odd = { ...from, target_month: 'Q4' };
		expect(initiativeEdit(odd, { ...odd, title: 'Orion 2' })).toEqual({ kind: 'send', body: { title: 'Orion 2' } });
	});
	it('treats a blank owner and spaces as the same', () => {
		const none = { ...from, owner: '' };
		expect(initiativeEdit(none, { ...none, owner: '  ' })).toEqual({ kind: 'unchanged' });
	});
	it('starts from the loaded initiative, absent fields as empty text', () => {
		const i = initiative({ key: 'ABC-1', title: 'Orion', horizon: 'Now', version: 3 });
		expect(initiativeDetails(i)).toEqual({ title: 'Orion', target_month: '', owner: '' });
	});
});

describe('epicEdit', () => {
	const e = epic({ key: 'ABC-2', title: 'E', initiative: 'ABC-1' });
	it('moves an epic to none, to another initiative, or leaves it', () => {
		const from = epicDetails(e);
		expect(from).toEqual({ title: 'E', initiative: 'ABC-1' });
		expect(epicEdit(from, { ...from, initiative: '' })).toEqual({ kind: 'send', body: { initiative: null } });
		expect(epicEdit(from, { ...from, initiative: 'ABC-5' })).toEqual({ kind: 'send', body: { initiative: 'ABC-5' } });
		expect(epicEdit(from, { ...from })).toEqual({ kind: 'unchanged' });
	});
	it('sends a changed title trimmed and refuses a blank one', () => {
		const from = epicDetails(e);
		expect(epicEdit(from, { ...from, title: ' E2 ' })).toEqual({ kind: 'send', body: { title: 'E2' } });
		expect(epicEdit(from, { ...from, title: ' ' })).toEqual({ kind: 'invalid', error: 'Title must not be blank.' });
	});
	it('starts an epic without an initiative as empty text', () => {
		expect(epicDetails({ ...e, initiative: null })).toEqual({ title: 'E', initiative: '' });
	});
});

describe('decisionEdit', () => {
	const from = { title: 'Pick', decider: 'dan@example.com', due: '2026-11-30', reversibility: '' as const };
	it.each([
		['classified one-way', { reversibility: 'one_way' }, { kind: 'send', body: { reversibility: 'one_way' } }],
		['the classification cleared', { reversibility: '' }, { kind: 'send', body: { reversibility: null } }, { reversibility: 'two_way' }],
		['the due date cleared', { due: '' }, { kind: 'send', body: { due: null } }],
		['another due date', { due: '2026-12-01' }, { kind: 'send', body: { due: '2026-12-01' } }],
		['the decider', { decider: ' bob@example.com ' }, { kind: 'send', body: { decider: 'bob@example.com' } }],
		['the title changed and re-spaced', { title: ' Pick 2 ' }, { kind: 'send', body: { title: 'Pick 2' } }],
		['a blank title', { title: ' ' }, { kind: 'invalid', error: 'Title and decider must not be blank.' }],
		['a blank decider', { decider: ' ' }, { kind: 'invalid', error: 'Title and decider must not be blank.' }],
		['nothing', {}, { kind: 'unchanged' }]
	] as [string, Partial<DecisionDraft>, unknown, Partial<DecisionDraft>?][])('%s', (_, patch, want, base = {}) =>
		expect(decisionEdit({ ...from, ...base }, { ...from, ...patch })).toEqual(want)
	);
});

describe('decisionDraft', () => {
	const d = decision({ id: 1, title: 'Pick', decider: 'dan@example.com', due: '2026-11-30' });
	it('starts from the loaded decision, an absent due date as empty text', () => {
		expect(decisionDraft(d)).toEqual({ title: 'Pick', decider: 'dan@example.com', due: '2026-11-30', reversibility: '' });
		expect(decisionDraft({ ...d, due: null })).toEqual({ title: 'Pick', decider: 'dan@example.com', due: '', reversibility: '' });
	});
	it('starts from the decision\'s reversibility, unset as empty text', () => {
		expect(decisionDraft({ ...d, reversibility: 'one_way' }).reversibility).toBe('one_way');
		expect(decisionDraft({ ...d, reversibility: null }).reversibility).toBe('');
	});
});

describe('fieldsText and mergeDraft', () => {
	it('compares fields by value, whatever their order', () => {
		expect(fieldsText({ a: '1', b: '2' })).toBe(fieldsText({ b: '2', a: '1' }));
		expect(fieldsText({ a: '1', b: '2' })).not.toBe(fieldsText({ a: '1', b: '3' }));
	});
	it("keeps the user's changed fields and takes the reloaded value of every other", () => {
		const from: DecisionDraft = { title: 'A', decider: 'x@example.com', due: '', reversibility: '' };
		const draft: DecisionDraft = { title: 'A2', decider: 'x@example.com', due: '', reversibility: '' };
		const server: DecisionDraft = { title: 'A by a colleague', decider: 'y@example.com', due: '2026-12-01', reversibility: 'one_way' };
		expect(mergeDraft(DECISION_SAME, from, draft, server)).toEqual({
			title: 'A2', decider: 'y@example.com', due: '2026-12-01', reversibility: 'one_way'
		});
	});
});

// The retry after a 409, for each editor: the user changed one field for real and touched another only in a way the
// edit helper treats as no change (re-spaced, re-cased); a colleague changed that second field meanwhile. The first
// send omits it, so the retry must too: merge with the editor's own table, then diff against the reloaded values.
describe('a retry after a conflict never reverts a colleague', () => {
	it('initiative details', () => {
		const from = { title: 'A', target_month: '2026-11', owner: 'dan@example.com' };
		const draft = { title: ' A ', target_month: '2026-12', owner: 'Dan@Example.com' };
		expect(initiativeEdit(from, draft)).toEqual({ kind: 'send', body: { target_month: '2026-12' } });
		const server = { title: 'B', target_month: '2026-11', owner: 'eve@example.com' };
		const merged = mergeDraft(INITIATIVE_SAME, from, draft, server);
		expect(initiativeEdit(server, merged)).toEqual({ kind: 'send', body: { target_month: '2026-12' } });
	});
	it('epic details', () => {
		const from = { title: 'E', initiative: 'ABC-1' };
		const draft = { title: ' E ', initiative: '' };
		const server = { title: 'E by a colleague', initiative: 'ABC-1' };
		const merged = mergeDraft(EPIC_SAME, from, draft, server);
		expect(epicEdit(server, merged)).toEqual({ kind: 'send', body: { initiative: null } });
	});
	it('a decision', () => {
		const from: DecisionDraft = { title: 'Pick', decider: 'dan@example.com', due: '', reversibility: '' };
		const draft: DecisionDraft = { title: 'Pick ', decider: ' dan@example.com', due: '2026-12-01', reversibility: '' };
		const server: DecisionDraft = { title: 'Pick a queue', decider: 'bob@example.com', due: '', reversibility: 'one_way' };
		const merged = mergeDraft(DECISION_SAME, from, draft, server);
		expect(decisionEdit(server, merged)).toEqual({ kind: 'send', body: { due: '2026-12-01' } });
	});
	it("a decision's classification", () => {
		const from: DecisionDraft = { title: 'Pick', decider: 'dan@example.com', due: '', reversibility: '' };
		const draft: DecisionDraft = { ...from, reversibility: 'two_way' };
		const server: DecisionDraft = { ...from, title: 'Pick a queue' };
		const merged = mergeDraft(DECISION_SAME, from, draft, server);
		expect(merged).toEqual({ ...server, reversibility: 'two_way' });
		expect(decisionEdit(server, merged)).toEqual({ kind: 'send', body: { reversibility: 'two_way' } });
	});
});
