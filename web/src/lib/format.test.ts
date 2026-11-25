import { describe, expect, it } from 'vitest';
import { age, dateLabel, dayLabel, errorMessage, month, sameEmail, short } from './format.ts';
import words from './promise.fixture.json';

const now = Date.parse('2026-10-26T12:00:00Z');
const ago = (days: number) => new Date(now - days * 86_400_000).toISOString();

describe('age', () => {
	const minsAgo = (m: number) => new Date(now - m * 60_000).toISOString();
	it.each([
		[0, '0m'],
		[1, '1m'],
		[59, '59m'],
		[60, '1h'],
		[61, '1h'],
		[23 * 60 + 59, '23h'],
		[24 * 60, '1d']
	])('%i minutes ago is %s', (mins, expected) => expect(age(minsAgo(mins), now)).toBe(expected));

	it.each([
		[1, '1d'],
		[3, '3d'],
		[13, '13d'],
		[14, '2w'],
		[14 * 1, '2w'],
		[59, '8w'],
		[60, '2mo'],
		[150, '5mo']
	])('%i days ago is %s', (days, expected) => expect(age(ago(days), now)).toBe(expected));

	it('clamps a future date to 0m', () => expect(age(ago(-5), now)).toBe('0m'));
	it('returns "?" for an invalid date', () => expect(age('not a date', now)).toBe('?'));
});

describe('month', () => {
	it('formats year-month', () => expect(month('2026-11')).toBe('Nov 2026'));
	it('returns anything else unchanged', () => {
		expect(month('2026-13')).toBe('2026-13');
		expect(month('soon')).toBe('soon');
	});
});

describe('dayLabel', () => {
	// Built from the string's parts, so no time zone can shift the day.
	it('formats a date as day and month', () => expect(dayLabel('2026-11-20')).toBe('20 Nov'));
	it('drops the leading zero', () => expect(dayLabel('2026-01-01')).toBe('1 Jan'));
	it('returns anything else unchanged', () => {
		expect(dayLabel('2026-13-01')).toBe('2026-13-01');
		expect(dayLabel('2026-11')).toBe('2026-11');
		expect(dayLabel('soon')).toBe('soon');
	});
});

describe('dateLabel', () => {
	it('adds the year to the day label', () => expect(dateLabel('2026-01-01')).toBe('1 Jan 2026'));
	it('returns anything else unchanged', () => expect(dateLabel('2026-13-01')).toBe('2026-13-01'));
});

describe('short', () => {
	it('takes the local part', () => expect(short('dan@example.com')).toBe('dan'));
	it('leaves a non-address unchanged', () => expect(short('dan')).toBe('dan'));
});

describe('sameEmail', () => {
	it('compares case-insensitively and without surrounding whitespace', () => {
		expect(sameEmail('Ana@X.se', ' ana@x.se ')).toBe(true);
		expect(sameEmail('a@b.se', 'c@b.se')).toBe(false);
	});
});

describe('errorMessage', () => {
	it('shows an Error by its message and anything else as text', () => {
		expect(errorMessage(new Error('boom'))).toBe('boom');
		expect(errorMessage('plain')).toBe('plain');
		expect(errorMessage(42)).toBe('42');
	});
});

describe('actionLabel', () => {
	it('names the file of an attachment entry', async () => {
		const { actionLabel } = await import('./format.ts');
		expect(actionLabel('attachment_added', { attachment_id: 'x', filename: 'shot.png' })).toBe('added attachment shot.png');
		expect(actionLabel('attachment_removed', { filename: 'shot.png' })).toBe('deleted attachment shot.png');
		expect(actionLabel('attachment_removed', null)).toBe('deleted attachment');
	});

	// Catches an epic's move between initiatives read as a bare "edited" (decision A of 2026-11-24: the move changes what
	// the epic's stories wait on, so the log says it and the views read it).
	it('says which initiative an edited epic left and joined', async () => {
		const { actionLabel, entryAction } = await import('./format.ts');
		const moved = (from: string | null, to: string | null) => ({ fields: ['initiative'], from: { initiative: from }, to: { initiative: to } });
		expect(actionLabel('edited', moved('ABC-3', 'ABC-9'))).toBe('edited and moved from initiative ABC-3 to ABC-9');
		expect(actionLabel('edited', moved(null, 'ABC-9'))).toBe('edited and moved to initiative ABC-9');
		expect(actionLabel('edited', moved('ABC-3', null))).toBe('edited and taken out of initiative ABC-3');
		expect(entryAction('edited', { ...moved(null, 'ABC-9'), client: 'claude.ai' })).toBe('edited and moved to initiative ABC-9 via claude.ai');
		// A move that also changed the title says the move: `fields` names every changed field.
		expect(actionLabel('edited', { ...moved('ABC-3', 'ABC-9'), fields: ['title', 'initiative'] })).toBe('edited and moved from initiative ABC-3 to ABC-9');
		// Every other edit has no words of its own and reads as "edited": no payload, another item's edit, an epic's
		// retitle or new brief, a required decision's edit (the same shape, other fields), and a payload that is not
		// the shape at all.
		const decision = { fields: ['decider'], from: { decider: 'dan@example.com' }, to: { decider: 'bob@example.com' } };
		for (const plain of [null, {}, { client: 'claude.ai' }, { fields: ['title', 'brief'] }, decision, moved(null, null), { fields: 'initiative' }, { fields: ['initiative'] }, { fields: ['initiative'], from: 7, to: 'ABC-9' }]) {
			expect(actionLabel('edited', plain)).toBeNull();
		}
	});

	// The account activity page's label() delegates to actionLabel, so these are its words too (it has no test file). The
	// rows live in promise.fixture.json, which the TUI ticker's tests (crates/reeve/src/promise.rs) read too.
	const activityRows = words.activity as { label: string; action: string; payload: unknown; plain: string; keyed: string }[];
	it.each<[string, string, unknown, string, string]>(activityRows.map((r) => [r.label, r.action, r.payload, r.plain, r.keyed]))('%s', async (_, action, payload, plain, keyed) => {
		const { actionLabel } = await import('./format.ts');
		expect(actionLabel(action, payload)).toBe(plain);
		expect(actionLabel(action, payload, 'ABC-12')).toBe(keyed);
	});
	it('names both items and the name between them for an item link entry, as stored', async () => {
		const { actionLabel } = await import('./format.ts');
		const payload = { type: 'follows', from: 'ABC-12', to: 'ABC-7' };
		expect(actionLabel('item_link_added', payload)).toBe('added item link: ABC-12 follows ABC-7');
		expect(actionLabel('item_link_removed', payload)).toBe('removed item link: ABC-12 follows ABC-7');
		expect(actionLabel('item_link_added', { type: 'relates_to', from: 'ABC-3', to: 'ABC-9' })).toBe('added item link: ABC-3 relates to ABC-9');
		// The entry is on both items, so the words are the same whichever item the note is about.
		expect(actionLabel('item_link_added', payload, 'ABC-7')).toBe('added item link: ABC-12 follows ABC-7');
		// A field missing, or a name this client does not know (it has no wording for it): the words without the link.
		for (const short of [null, {}, { type: 'follows', from: 'ABC-12' }, { type: 7, from: 'ABC-12', to: 'ABC-7' }, { type: 'blocks', from: 'ABC-12', to: 'ABC-7' }]) {
			expect(actionLabel('item_link_added', short)).toBe('added item link');
			expect(actionLabel('item_link_removed', short)).toBe('removed item link');
		}
	});
	it('names the title of a web link entry, and the URL too once the web link is removed', async () => {
		const { actionLabel } = await import('./format.ts');
		const payload = { id: 4, title: 'Scenario page', url: 'https://example.com/wiki/scenario' };
		expect(actionLabel('web_link_added', payload)).toBe('added web link Scenario page');
		// Removed, it is listed nowhere any more: the entry is the one place left that says where it pointed.
		expect(actionLabel('web_link_removed', payload)).toBe('removed web link Scenario page (https://example.com/wiki/scenario)');
		expect(actionLabel('web_link_removed', { title: 'Scenario page' })).toBe('removed web link Scenario page');
		expect(actionLabel('web_link_added', null)).toBe('added web link');
		expect(actionLabel('web_link_removed', { title: '' })).toBe('removed web link');
	});
	it('has no words for any other action, which reads as plainAction', async () => {
		const { actionLabel, plainAction } = await import('./format.ts');
		for (const a of ['moved', 'goal_linked', 'override']) expect(actionLabel(a, null, 'ABC-12')).toBeNull();
		expect(plainAction('goal_linked')).toBe('goal linked');
	});
});

describe('entryAction', () => {
	it('adds the client a write was made through, to worded and plain actions alike', async () => {
		const { entryAction } = await import('./format.ts');
		expect(entryAction('moved', { from: 'todo', to: 'in_progress', client: 'claude.ai' })).toBe('moved via claude.ai');
		expect(entryAction('goal_linked', { client: 'chatgpt.com' })).toBe('goal linked via chatgpt.com');
		expect(entryAction('attachment_added', { filename: 'shot.png', client: 'claude.ai' })).toBe('added attachment shot.png via claude.ai');
		expect(entryAction('web_link_added', { title: 'Scenario page', client: 'claude.ai' })).toBe('added web link Scenario page via claude.ai');
	});
	it('reads as before without a client', async () => {
		const { entryAction } = await import('./format.ts');
		expect(entryAction('moved', { from: 'todo', to: 'in_progress' })).toBe('moved');
		expect(entryAction('attachment_added', { filename: 'shot.png' })).toBe('added attachment shot.png');
		for (const payload of [null, undefined, 'text', {}, { client: '' }, { client: 7 }, { client: null }]) expect(entryAction('edited', payload)).toBe('edited');
	});
});
