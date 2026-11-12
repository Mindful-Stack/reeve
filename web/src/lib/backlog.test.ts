// web/src/lib/backlog.test.ts
import { describe, expect, it } from 'vitest';
import type { Reason, Story } from '#lib/api/types.ts';
import { epic as epicDto, initiative, withReadiness } from '#lib/test/fixtures.ts';
import { activeFilterCount, backlogFilters, backlogState, backlogView, FILTER_PARAMS, groupBacklog, stateParam, toStoryQuery, withoutFilters, withPromised } from './backlog.ts';

const ini = (key: string, horizon: string, title = key) => initiative({ key, title, horizon });
const epic = (key: string, initiative: string | null) => epicDto({ key, title: `Epic ${key}`, initiative });
const row = (key: string, rank: number, opts: { epic?: string; kind?: Story['kind']; ready?: boolean; untriaged?: boolean } = {}) => {
	const reasons: Reason[] = opts.ready ? [] : opts.untriaged ? [{ reason: 'untriaged' }, { reason: 'no_criteria' }] : [{ reason: 'no_criteria' }];
	return withReadiness({ key, rank, title: key, kind: opts.kind ?? 'story', epic: opts.epic ?? null, delivery: 'todo' }, reasons);
};

describe('groupBacklog', () => {
	const buckets = ['Now', 'Next'];
	const initiatives = [ini('I-3', 'Next', 'Search'), ini('I-2', 'Next', 'Billing'), ini('I-1', 'Now', 'Onboarding')];
	const epics = [epic('E-1', 'I-1'), epic('E-2', 'I-1'), epic('E-3', null), epic('E-4', 'I-2')];
	const todo = [
		row('S-5', 5, { epic: 'E-1', ready: true }),
		row('S-1', 1, { kind: 'chore', untriaged: true }),
		row('S-2', 2, { epic: 'E-1' }),
		row('S-3', 3, { kind: 'story', untriaged: true }),
		row('S-4', 4, { epic: 'E-3', ready: true }),
		row('S-0', 0, { kind: 'chore', untriaged: true }),
		row('S-6', 6, { epic: 'E-9' })
	];
	const b = groupBacklog(buckets, initiatives, epics, todo);

	it("No epic first: every epic-less row in one run, in the order given (the server's), all counted untriaged", () => {
		expect(b.noEpic.map((r) => r.story.key)).toEqual(['S-1', 'S-3', 'S-0']);
		expect(b.untriaged).toBe(3);
	});
	it('initiatives in planning order, every epic listed, an empty one says so', () => {
		expect(b.sections.map((s) => s.title)).toEqual(['Onboarding', 'Billing', 'Search', 'No initiative']);
		const i1 = b.sections[0].epics;
		expect(i1.map((e) => e.epic.key)).toEqual(['E-1', 'E-2']);
		expect(i1[0].rows.map((r) => r.story.key)).toEqual(['S-5', 'S-2']); // the server's order: never re-sorted
		expect(i1[0].summary).toBe('1 of 2 ready');
		expect(i1[0].tone).toBe('warn');
		expect(i1[1].tone).toBe('muted');
		expect(i1[1].summary).toBe('No stories waiting');
		expect(b.sections[1].epics[0].summary).toBe('No stories waiting'); // E-4, no todo stories
		expect(b.sections[1].epics[0].tone).toBe('muted');
		expect(b.sections[2].epics).toEqual([]); // an initiative without epics is kept
		expect(b.sections[2].initiative?.key).toBe('I-3');
	});
	it('stories whose epic is unknown are returned as orphans', () => {
		expect(b.orphans.map((r) => r.story.key)).toEqual(['S-6']);
	});
	it('epics without an initiative go under No initiative', () => {
		expect(b.sections[3].epics.map((e) => e.epic.key)).toEqual(['E-3']);
		expect(b.sections[3].epics[0].tone).toBe('ready');
	});
	it('filtered: omits epics and initiatives with no rows and counts the matches', () => {
		const f = groupBacklog(buckets, initiatives, epics, [row('S-2', 2, { epic: 'E-1' }), row('S-5', 5, { epic: 'E-1', ready: true }), row('S-1', 1, { untriaged: true }), row('S-4', 4, { epic: 'E-3' })], { filtered: true });
		expect(f.sections.map((s) => s.title)).toEqual(['Onboarding', 'No initiative']);
		expect(f.sections[0].epics.map((e) => [e.epic.key, e.summary])).toEqual([['E-1', '2 matches']]);
		expect(f.sections[1].epics.map((e) => [e.epic.key, e.summary])).toEqual([['E-3', '1 match']]);
		expect(f.noEpic.map((r) => r.story.key)).toEqual(['S-1']);
		expect(groupBacklog(buckets, initiatives, epics, [], { filtered: true }).sections).toEqual([]);
	});
	it('outside Todo: every epic is listed with its story count, not its readiness', () => {
		const o = groupBacklog(buckets, initiatives, epics, [row('S-2', 2, { epic: 'E-1' }), row('S-5', 5, { epic: 'E-1', ready: true })], { todo: false });
		expect(o.sections.map((s) => s.title)).toEqual(['Onboarding', 'Billing', 'Search', 'No initiative']);
		expect(o.sections[0].epics.map((e) => [e.epic.key, e.summary, e.tone])).toEqual([
			['E-1', '2 stories', 'muted'],
			['E-2', 'No stories', 'muted']
		]);
		const f = groupBacklog(buckets, initiatives, epics, [row('S-2', 2, { epic: 'E-1' })], { filtered: true, todo: false });
		expect(f.sections[0].epics.map((e) => e.summary)).toEqual(['1 match']);
	});
});

describe('activeFilterCount and withoutFilters', () => {
	const at = (search: string) => new URL(`http://localhost/backlog${search}`);
	it('count the filters, not state, view or text; Mine only while signed in', () => {
		expect(activeFilterCount(at('?q=a&state=open&view=rank'), 'a@example.com')).toBe(0);
		expect(activeFilterCount(at('?kind=bug&mine=1&urgent=1'), 'a@example.com')).toBe(3);
		expect(activeFilterCount(at('?kind=bug&mine=1'), null)).toBe(1);
	});
	it('Clear filters removes the filters and keeps the text, state and view', () => {
		expect(withoutFilters(at('?q=zebra&state=open&view=relevance&kind=bug&mine=1&story=S-1')).search).toBe('?q=zebra&state=open&view=relevance&story=S-1');
	});
});

describe('toStoryQuery', () => {
	const at = (search: string) => new URL(`http://localhost/backlog${search}`);
	const me = 'a@example.com';

	it('is null for the plain backlog: the unfiltered todo list is the rows', () => {
		expect(toStoryQuery(at(''), me)).toBeNull();
		expect(toStoryQuery(at('?group=rank'), me)).toBeNull();
		expect(toStoryQuery(at('?view=rank&state=todo'), me)).toBeNull();
		expect(toStoryQuery(at('?story=S-1'), me)).toBeNull();
	});
	it('defaults state to todo, also with text (the header box supplies state=open)', () => {
		expect(toStoryQuery(at('?q=a'), me)).toEqual({ q: 'a', delivery: 'todo' });
		expect(toStoryQuery(at('?q=%20a%20'), me)).toEqual({ q: 'a', delivery: 'todo' });
		expect(toStoryQuery(at('?q=%20%20'), me)).toBeNull();
	});
	it('maps state to delivery or done, and relevance to sort only with text', () => {
		expect(toStoryQuery(at('?q=a&state=open&view=relevance'), me)).toEqual({ q: 'a', done: 'none', sort: 'relevance' });
		expect(toStoryQuery(at('?state=all'), me)).toEqual({ done: 'all' });
		expect(toStoryQuery(at('?state=open'), me)).toEqual({ done: 'none' });
		expect(toStoryQuery(at('?state=zz'), me)).toBeNull();
		expect(toStoryQuery(at('?view=relevance'), me)).toBeNull();
		expect(toStoryQuery(at('?view=relevance&state=all'), me)).toEqual({ done: 'all' });
		expect(toStoryQuery(at('?q=a&view=rank'), me)).toEqual({ q: 'a', delivery: 'todo' });
	});
	it('sends mine as the owner only while signed in', () => {
		expect(toStoryQuery(at('?mine=1'), me)).toEqual({ delivery: 'todo', owner: me });
		expect(toStoryQuery(at('?mine=1'), undefined)).toBeNull();
		expect(toStoryQuery(at('?mine=1&state=open'), null)).toEqual({ done: 'none' });
	});
	it('drops an unknown kind and passes the other filters through', () => {
		expect(toStoryQuery(at('?kind=zz'), me)).toBeNull();
		expect(toStoryQuery(at('?kind=bug&epic=E-1&initiative=I-1&label=ux&ready=1&untriaged=1&urgent=1'), me)).toEqual({
			delivery: 'todo',
			kind: 'bug',
			epic: 'E-1',
			initiative: 'I-1',
			label: 'ux',
			ready: true,
			untriaged: true,
			urgent: true
		});
		expect(toStoryQuery(at('?ready=0&urgent=true'), me)).toBeNull();
	});
});

describe('backlogState and backlogView', () => {
	const at = (search: string) => new URL(`http://localhost/backlog${search}`);
	it('read the URL with their defaults', () => {
		expect(backlogState(at(''))).toBe('todo');
		expect(backlogState(at('?state=open'))).toBe('open');
		expect(backlogState(at('?state=all'))).toBe('all');
		expect(backlogState(at('?state=zz'))).toBe('todo');
		expect(backlogView(at(''))).toBe('epic');
		expect(backlogView(at('?group=rank'))).toBe('rank');
		expect(backlogView(at('?view=rank'))).toBe('rank');
		expect(backlogView(at('?view=epic&group=rank'))).toBe('epic');
		expect(backlogView(at('?q=a&view=relevance'))).toBe('relevance');
	});
	it('shows relevance without text as ranked', () => {
		expect(backlogView(at('?view=relevance'))).toBe('rank');
		expect(backlogView(at('?view=relevance&q=%20'))).toBe('rank');
	});
});

describe('the promised filter', () => {
	const at = (search: string) => new URL(`http://localhost/backlog${search}`);
	const me = 'a@example.com';

	it('accepts the four values and ignores any other', () => {
		for (const v of ['this_sprint', 'next_sprint', 'overdue', 'any'] as const) expect(backlogFilters(at(`?promised=${v}`), me)).toEqual({ promised: v });
		expect(backlogFilters(at('?promised=bogus'), me)).toEqual({});
		expect(backlogFilters(at('?promised='), me)).toEqual({});
	});
	it('is a filter param: counted, and removed by Clear filters', () => {
		expect(FILTER_PARAMS).toContain('promised');
		expect(activeFilterCount(at('?promised=overdue'), me)).toBe(1);
		expect(withoutFilters(at('?promised=overdue&state=all')).search).toBe('?state=all');
	});
	it('answers open when the URL names no state, so an unfinished story in progress still shows', () => {
		expect(backlogState(at('?promised=this_sprint'))).toBe('open');
		expect(backlogState(at('?promised=this_sprint&state=zz'))).toBe('open');
		expect(backlogState(at('?promised=bogus'))).toBe('todo');
		expect(toStoryQuery(at('?promised=this_sprint'), me)).toEqual({ done: 'none', promised: 'this_sprint' });
		expect(toStoryQuery(at('?promised=this_sprint'), me)).not.toHaveProperty('delivery');
	});
	it('keeps an explicit state', () => {
		expect(backlogState(at('?promised=any&state=todo'))).toBe('todo');
		expect(toStoryQuery(at('?promised=any&state=todo'), me)).toEqual({ delivery: 'todo', promised: 'any' });
		expect(toStoryQuery(at('?promised=any&state=all'), me)).toEqual({ done: 'all', promised: 'any' });
	});
	it('clearing it (Clear filters, or the select\'s empty option) also drops state=todo, which only the filter needed', () => {
		expect(withoutFilters(at('?promised=any&state=todo&q=a')).search).toBe('?q=a');
		expect(withoutFilters(at('?promised=any&state=open')).search).toBe('?state=open');
		expect(withPromised(at('?promised=any&state=todo&kind=bug'), null).search).toBe('?kind=bug');
		expect(withPromised(at('?promised=any&state=all'), null).search).toBe('?state=all');
		expect(withPromised(at('?state=todo'), 'overdue').search).toBe('?state=todo&promised=overdue');
		expect(withPromised(at(''), 'overdue').search).toBe('?promised=overdue');
	});
	it('the State control names Todo explicitly while the filter would otherwise answer open', () => {
		expect(stateParam('todo', at('?promised=any'))).toBe('todo');
		expect(stateParam('todo', at(''))).toBeNull();
		expect(stateParam('open', at('?promised=any'))).toBe('open');
		expect(stateParam('all', at(''))).toBe('all');
	});
});
