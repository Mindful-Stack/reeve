import { describe, expect, it } from 'vitest';
import { story } from '#lib/test/fixtures.ts';
import {
	armedFor, byKey, criteriaFromText, draftOf, duplicateCommentKey, duplicateText, followDraft, nextKey, placeEdit,
	rebaseAfterConflict, similarQuery, undoEdit, whereNow, withoutSelf
} from './triage.ts';

describe('byKey', () => {
	it('orders by the key number, not by text or by the server order', () => {
		const rows = ['GRA-100', 'GRA-9', 'GRA-20'].map((key) => story({ key }));
		expect(byKey(rows).map((s) => s.key)).toEqual(['GRA-9', 'GRA-20', 'GRA-100']);
	});
});

describe('criteriaFromText', () => {
	const base = [{ text: 'Exports a PDF', done: true }, { text: 'Shows a preview', done: false }];
	it('keeps done for an unchanged line at the same position, drops blank lines and trims', () => {
		expect(criteriaFromText('  Exports a PDF \n\nShows a preview\nNew one', base)).toEqual([
			{ text: 'Exports a PDF', done: true },
			{ text: 'Shows a preview', done: false },
			{ text: 'New one', done: false }
		]);
	});
	it('a moved or edited line starts not done', () => {
		expect(criteriaFromText('Shows a preview\nExports a PDF', base)).toEqual([
			{ text: 'Shows a preview', done: false },
			{ text: 'Exports a PDF', done: false }
		]);
	});
});

describe('placeEdit', () => {
	const s = story({ key: 'GRA-5', kind: 'bug', title: 'Blank PDF', criteria: [{ text: 'Renders', done: true }], version: 7 });
	it('refuses without an epic', () => {
		expect(placeEdit(draftOf(s))).toEqual({ kind: 'invalid', error: 'Pick an epic first.' });
	});
	it('sends the version, the epic, and only the fields that changed', () => {
		const d = draftOf(s);
		d.fields.epic = 'GRA-7';
		expect(placeEdit(d)).toEqual({ kind: 'send', body: { version: 7, epic: 'GRA-7' } });
		d.fields.kind = 'story';
		d.fields.criteria = 'Renders\nKeeps fonts';
		expect(placeEdit(d)).toEqual({
			kind: 'send',
			body: { version: 7, epic: 'GRA-7', kind: 'story', criteria: [{ text: 'Renders', done: true }, { text: 'Keeps fonts', done: false }] }
		});
	});
	it('a re-spaced title or criterion is not a change', () => {
		const d = draftOf(s);
		d.fields.epic = 'GRA-7';
		d.fields.title = '  Blank PDF ';
		d.fields.criteria = 'Renders\n\n';
		expect(placeEdit(d)).toEqual({ kind: 'send', body: { version: 7, epic: 'GRA-7' } });
	});
	it('refuses a blank title', () => {
		const d = draftOf(s);
		d.fields.epic = 'GRA-7';
		d.fields.title = '  ';
		expect(placeEdit(d)).toEqual({ kind: 'invalid', error: 'Title must not be blank.' });
	});
});

describe('followDraft and rebaseAfterConflict', () => {
	const s = story({ key: 'GRA-5', title: 'Blank PDF', version: 3 });
	it('follow: the base advances and untouched fields take the server value when the user\'s fields are unchanged there', () => {
		const d = draftOf(s);
		d.fields.epic = 'GRA-7';
		d.duplicate = 'GRA-3';
		const f = followDraft(d, story({ key: 'GRA-5', title: 'Blank PDF export', version: 4 }));
		expect(f.base.version).toBe(4);
		expect(f.fields).toMatchObject({ epic: 'GRA-7', title: 'Blank PDF export' });
		expect(f.duplicate).toBe('GRA-3');
	});
	it('follow: an unchanged item leaves the draft as it is, a line being typed included', () => {
		const d = draftOf(story({ key: 'GRA-5', criteria: [{ text: 'Renders', done: false }], version: 3 }));
		d.fields.criteria = 'Renders\n';
		expect(followDraft(d, story({ key: 'GRA-5', criteria: [{ text: 'Renders', done: false }], version: 3 }))).toBe(d);
	});
	it('follow: a clash keeps the old base, so Place conflicts instead of overwriting', () => {
		const d = draftOf(s);
		d.fields.title = 'Mine';
		const f = followDraft(d, story({ key: 'GRA-5', title: 'Theirs', version: 4 }));
		expect(f.base.version).toBe(3);
		expect(f.fields.title).toBe('Mine');
	});
	it('rebase: keeps the user\'s changes, takes the server\'s others, and names the fields both changed', () => {
		const d = draftOf(s);
		d.fields.epic = 'GRA-7';
		d.fields.title = 'Mine';
		const server = story({ key: 'GRA-5', title: 'Theirs', kind: 'chore', version: 5 });
		const r = rebaseAfterConflict(d, server);
		expect(r.draft.base.version).toBe(5);
		expect(r.draft.fields).toMatchObject({ epic: 'GRA-7', title: 'Mine', kind: 'chore' });
		expect(r.clashes).toEqual([{ field: 'title', value: 'Theirs' }]);
	});
});

describe('undoEdit', () => {
	it('restores the base value of every field the Place sent, at the placed version', () => {
		const s = story({ key: 'GRA-5', kind: 'bug', criteria: [{ text: 'Renders', done: true }], version: 3 });
		const d = draftOf(s);
		const sent = { version: 3, epic: 'GRA-7', kind: 'story' as const, criteria: [{ text: 'New', done: false }] };
		expect(undoEdit(story({ key: 'GRA-5', version: 4 }), d, sent)).toEqual({
			version: 4,
			epic: null,
			kind: 'bug',
			criteria: [{ text: 'Renders', done: true }]
		});
	});
});

describe('small helpers', () => {
	it('nextKey: the next, else the previous, else null', () => {
		expect(nextKey(['A-1', 'A-2', 'A-3'], 'A-2')).toBe('A-3');
		expect(nextKey(['A-1', 'A-2', 'A-3'], 'A-3')).toBe('A-2');
		expect(nextKey(['A-1'], 'A-1')).toBeNull();
	});
	it('duplicate text and comment key: one comment per item and duplicate, whatever the version', () => {
		expect(duplicateText('GRA-3')).toBe('Duplicate of GRA-3.');
		expect(duplicateCommentKey('GRA-5', 'GRA-3')).toBe('triage-dup:GRA-5:GRA-3');
	});
	it('withoutSelf drops the item itself and keeps at most five', () => {
		const hits = ['GRA-5', 'A-1', 'A-2', 'A-3', 'A-4', 'A-5'].map((key) => ({ key, title: key, kind: 'story' as const, delivery: 'todo' as const }));
		expect(withoutSelf(hits, 'GRA-5').map((h) => h.key)).toEqual(['A-1', 'A-2', 'A-3', 'A-4', 'A-5']);
	});
	it('similarQuery: the trimmed title within 8..200 characters, counted as the server counts (code points), else null', () => {
		expect(similarQuery('  short ')).toBeNull();
		expect(similarQuery(' Blank PDF ')).toBe('Blank PDF');
		expect(similarQuery('x'.repeat(201))).toBeNull();
		expect(similarQuery('😀'.repeat(150))).toBe('😀'.repeat(150)); // 300 UTF-16 units, 150 characters
	});
	it('whereNow names the epic, or done and the resolution', () => {
		expect(whereNow(story({ key: 'GRA-5', epic: 'GRA-7' }))).toBe('Placed elsewhere: GRA-5 is under GRA-7.');
		expect(whereNow(story({ key: 'GRA-5', delivery: 'done', resolution: 'dropped' }))).toBe('GRA-5 is done (dropped).');
	});
	it('armedFor: the same item and version within 3 seconds', () => {
		const arm = { key: 'GRA-5', version: 2, at: 1000 };
		expect(armedFor(arm, 'GRA-5', 2, 3999)).toBe(true);
		expect(armedFor(arm, 'GRA-5', 2, 4000)).toBe(false);
		expect(armedFor(arm, 'GRA-5', 3, 2000)).toBe(false);
		expect(armedFor(arm, 'GRA-6', 2, 2000)).toBe(false);
		expect(armedFor(null, 'GRA-5', 2, 2000)).toBe(false);
	});
});
