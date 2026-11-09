// @vitest-environment jsdom
// The Report draft: kept across close and navigation, cleared by a create or Discard, restored after a reload for the
// same account only, and an uncertain send keeps its exact payload and key.
import { beforeEach, describe, expect, it } from 'vitest';
import { report, openReport, discardDraft, hasDraft, restoreDraft, templateFor, isUntouched, payloadOf, persist } from './report.svelte.ts';

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
		openReport({ path: '/board?story=ABC-12', label: 'Board', epic: null });
		expect(report.draft?.origin).toEqual({ path: '/epics/ABC-30', label: 'Epic ABC-30' });
		expect(report.draft?.epic?.key).toBe('ABC-30');
	});
	it('keeps an empty draft whose send is uncertain', () => {
		openReport({ path: '/board', label: 'Board', epic: null });
		report.draft!.uncertain = true;
		openReport({ path: '/epics/ABC-30', label: 'Epic ABC-30', epic: { key: 'ABC-30', title: 'Payments' } });
		expect(report.draft?.origin.path).toBe('/board');
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
		expect(restoreDraft('dan@example.com')).toBe(false); // and gone from storage
	});
});
