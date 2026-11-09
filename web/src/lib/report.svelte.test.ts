// @vitest-environment jsdom
// The Report draft: kept across close and navigation, cleared by a create or Discard, restored after a reload for the
// same account only, and an uncertain send keeps its exact payload and key.
import { beforeEach, describe, expect, it } from 'vitest';
import { report, openReport, discardDraft, hasDraft, restoreDraft, templateFor, isUntouched, payloadOf, payloadText, persist } from './report.svelte.ts';

beforeEach(() => {
	sessionStorage.clear();
	discardDraft();
});

describe('report draft', () => {
	it('captures the context once a draft has content, and takes the new context while it is empty', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		openReport({ path: '/epics/ABC-30', label: 'Epic ABC-30', epic: { key: 'ABC-30', title: 'Payments' } });
		expect(report.draft?.origin.path).toBe('/epics/ABC-30'); // nothing typed yet: the newer page wins
		report.draft!.title = 'PDF export blank';
		const key = report.draft!.key.key;
		openReport({ path: '/board?story=ABC-12', label: 'Board', epic: null });
		expect(report.draft?.key.key).toBe(key);
		expect(report.draft?.origin).toEqual({ path: '/epics/ABC-30', label: 'Epic ABC-30' });
		expect(report.draft?.epic?.key).toBe('ABC-30');
	});
	it('keeps an empty draft whose send is uncertain', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.uncertain = true;
		const key = report.draft!.key.key;
		openReport({ path: '/epics/ABC-30', label: 'Epic ABC-30', epic: { key: 'ABC-30', title: 'Payments' } });
		expect(report.draft?.origin.path).toBe('/board');
		expect(report.draft?.key.key).toBe(key);
	});
	it('is a draft only once something was typed', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		expect(hasDraft()).toBe(false);
		report.draft!.title = 'PDF export blank';
		expect(hasDraft()).toBe(true);
	});
	it('treats a template as untouched and sends it empty', () => {
		expect(isUntouched(templateFor('bug'))).toBe(true);
		expect(isUntouched(templateFor('bug') + 'it broke')).toBe(false);
		openReport({ path: '/board', label: 'Board', epic: null });
		Object.assign(report.draft!, { title: 'PDF export blank', kind: 'bug', description: templateFor('bug') });
		expect(payloadOf(report.draft!).description).toBe('Reported from: [Board](/board)');
	});
	it('restores after a reload for the same account only', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.title = 'PDF export blank';
		persist('dan@example.com');
		discardDraft({ keepStorage: true });
		expect(restoreDraft('dan@example.com')).toBe(true);
		expect(report.draft?.title).toBe('PDF export blank');
		persist('dan@example.com');
		discardDraft({ keepStorage: true });
		expect(restoreDraft('eve@example.com')).toBe(false); // another account: dropped, never shown
		expect(report.draft).toBeNull();
		expect(restoreDraft('dan@example.com')).toBe(false); // and gone from storage
	});
	it('round-trips the key and the uncertain flag', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		Object.assign(report.draft!, { title: 'x', uncertain: true });
		report.draft!.key.sent = 'x';
		const saved = { ...report.draft!.key };
		persist('dan@example.com');
		discardDraft({ keepStorage: true });
		expect(restoreDraft('dan@example.com')).toBe(true);
		expect(report.draft?.key).toEqual(saved);
		expect(report.draft?.uncertain).toBe(true);
	});
	it('drops stored data that is not a draft', () => {
		sessionStorage.setItem('reeve.report-draft', JSON.stringify({ email: 'dan@example.com', draft: {} }));
		expect(restoreDraft('dan@example.com')).toBe(false);
		expect(sessionStorage.getItem('reeve.report-draft')).toBeNull();
		sessionStorage.setItem('reeve.report-draft', 'not json');
		expect(restoreDraft('dan@example.com')).toBe(false);
		expect(sessionStorage.getItem('reeve.report-draft')).toBeNull();
	});
	it('restores only a real draft, and persist clears storage for an empty one', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		persist('dan@example.com');
		expect(sessionStorage.getItem('reeve.report-draft')).toBeNull();
		report.draft!.title = 'x';
		persist('dan@example.com');
		report.draft!.title = '';
		sessionStorage.setItem('reeve.report-draft', JSON.stringify({ email: 'dan@example.com', draft: report.draft }));
		discardDraft({ keepStorage: true });
		expect(restoreDraft('dan@example.com')).toBe(false);
		expect(sessionStorage.getItem('reeve.report-draft')).toBeNull();
	});
	it('keeps the context once a kind is chosen', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.kind = 'bug';
		openReport({ path: '/sprint', label: 'Sprint', epic: null });
		expect(report.draft?.origin.path).toBe('/board');
	});
	it('treats a template with trailing whitespace changes as untouched', () => {
		expect(isUntouched(templateFor('bug') + '\n')).toBe(true);
	});
	it('appends the origin once after an edited description', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		Object.assign(report.draft!, { title: 't', kind: 'bug', description: 'it broke\n' });
		const d = payloadOf(report.draft!).description;
		expect(d).toBe('it broke\n\nReported from: [Board](/board)');
		expect(d.split('Reported from').length).toBe(2);
	});
	it('keeps the link well formed for awkward paths and labels', () => {
		openReport({ path: '/search?q=a)', label: 'A [b]', epic: null });
		Object.assign(report.draft!, { title: 't', kind: 'bug' });
		const d = payloadOf(report.draft!).description;
		expect(d).toContain('%29');
		expect(d).toContain('A \\[b\\]');
		expect(payloadText(report.draft!)).toBe(JSON.stringify(payloadOf(report.draft!)));
	});
	it('refuses a payload without a kind', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		expect(() => payloadOf(report.draft!)).toThrow('a report needs a kind');
	});
});
