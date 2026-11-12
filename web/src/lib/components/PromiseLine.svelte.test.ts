// @vitest-environment jsdom
// The promise line: every case is text in the DOM, and the epic's key links to the epic.
import { mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import type { EpicPromise, PromiseFact, StoryPromise } from '#lib/api/types.ts';
import PromiseLine from './PromiseLine.svelte';

const fact = (on: string, to: string | null = 'Acme'): PromiseFact => ({ on, to, by: 'Dana', at: '2026-11-01T09:00:00Z' });
const sp = (over: Partial<StoryPromise>): StoryPromise => ({
	own: fact('2026-11-20'),
	epic: null,
	on: '2026-11-20',
	holder: 'own',
	state: 'open',
	due: 'later',
	days: null,
	...over
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

const line = () => document.body.querySelector('[data-testid="promise-line"]')!;
const text = () => line().textContent?.replace(/\s+/g, ' ').trim();

describe('PromiseLine', () => {
	it.each<[string, StoryPromise | EpicPromise, string]>([
		['own only', sp({}), 'Promised 20 Nov to Acme · set by Dana'],
		['epic only', sp({ own: null, epic: { key: 'ABC-7', fact: fact('2026-11-20') }, holder: 'epic' }), 'Promised 20 Nov via epic ABC-7 to Acme · set by Dana'],
		[
			'both, epic earlier',
			sp({ own: fact('2026-11-30', 'Beta'), epic: { key: 'ABC-7', fact: fact('2026-11-18') }, on: '2026-11-18', holder: 'epic' }),
			'Promised 30 Nov to Beta. Epic ABC-7 is promised 18 Nov; the earlier date counts.'
		],
		[
			'both, own earlier',
			sp({ own: fact('2026-11-18', 'Beta'), epic: { key: 'ABC-7', fact: fact('2026-11-30') }, on: '2026-11-18' }),
			'Promised 18 Nov to Beta. Epic ABC-7 is promised 30 Nov.'
		],
		['overdue', sp({ state: 'overdue', due: null, days: 3 }), 'Promised 20 Nov to Acme · set by Dana · overdue 3 d'],
		['kept', sp({ state: 'kept', due: null }), 'Promised 20 Nov to Acme · set by Dana · kept'],
		['late', sp({ state: 'late', due: null, days: 3 }), 'Promised 20 Nov to Acme · set by Dana · done 3 d late'],
		['dropped', sp({ state: 'dropped', due: null }), 'Promised 20 Nov to Acme · set by Dana · dropped'],
		['epic page', { fact: fact('2026-11-20'), due: 'later', past: false }, 'Promised 20 Nov to Acme · set by Dana'],
		['epic page, past', { fact: fact('2026-11-20'), due: null, past: true }, 'Promised 20 Nov to Acme · set by Dana · past']
	])('%s', (_, promise, want) => {
		app = mount(PromiseLine, { target: document.body, props: { promise } });
		expect(text()).toBe(want);
		expect(document.body.querySelectorAll('[title]')).toHaveLength(0);
	});

	it('links the epic key', () => {
		app = mount(PromiseLine, {
			target: document.body,
			props: { promise: sp({ own: null, epic: { key: 'ABC-7', fact: fact('2026-11-20') }, holder: 'epic' }) }
		});
		const a = line().querySelector('a')!;
		expect(a.textContent).toBe('ABC-7');
		expect(a.getAttribute('href')).toBe('/epics/ABC-7');
	});
});
