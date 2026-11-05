import { describe, expect, it } from 'vitest';
import type { PrStatus, PullRequest } from '#lib/api/types.ts';
import { PR_LABEL, prLine, prSummary, safePrUrl } from './prs.ts';

const pr = (status: PrStatus, number = 1): PullRequest => ({
	repo: 'acme/web',
	number,
	url: `https://github.com/acme/web/pull/${number}`,
	title: 't',
	status,
	updated_at: '2026-10-26T12:00:00Z'
});

describe('prSummary', () => {
	it('is empty without pull requests', () => {
		expect(prSummary([])).toEqual({ label: '', status: null });
	});
	it('is the status of a single pull request', () => {
		expect(prSummary([pr('draft')])).toEqual({ label: PR_LABEL.draft, status: 'draft' });
	});
	it('picks merged > open > draft > closed', () => {
		const all: PrStatus[] = ['closed', 'draft', 'open', 'merged'];
		expect(prSummary(all.map((s, i) => pr(s, i))).status).toBe('merged');
		expect(prSummary([pr('closed'), pr('draft'), pr('open')]).status).toBe('open');
		expect(prSummary([pr('closed'), pr('draft')]).status).toBe('draft');
		expect(prSummary([pr('closed', 1), pr('closed', 2)]).status).toBe('closed');
	});
	it('counts several pull requests in the label', () => {
		expect(prSummary([pr('open', 1), pr('merged', 2)]).label).toBe('2 pull requests');
	});
});

describe('prLine', () => {
	it('reads repo#number · status', () => {
		expect(prLine(pr('open', 7))).toBe('acme/web#7 · open');
	});
});

describe('safePrUrl', () => {
	it('keeps http(s) urls', () => {
		expect(safePrUrl('https://github.com/acme/web/pull/1')).toBe('https://github.com/acme/web/pull/1');
	});
	it('rejects other schemes and relative urls', () => {
		expect(safePrUrl('javascript:alert(1)')).toBeNull();
		expect(safePrUrl('data:text/html,x')).toBeNull();
		expect(safePrUrl('/pull/1')).toBeNull();
	});
});
