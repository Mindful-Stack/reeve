import { describe, expect, it } from 'vitest';
import type { Token } from '#lib/api/types.ts';
import { liveBrowserSession } from './tokens.ts';

const now = new Date('2026-10-26T12:00:00Z');
const token = (over: Partial<Token>): Token => ({
	id: 1,
	kind: 'interactive',
	label: 'web session',
	created_at: '2026-10-01T00:00:00Z',
	expires_at: '2026-11-01T00:00:00Z',
	revoked_at: null,
	...over
});

describe('liveBrowserSession', () => {
	it('marks a live interactive web session', () => {
		expect(liveBrowserSession(token({}), now)).toBe(true);
		expect(liveBrowserSession(token({ expires_at: null }), now)).toBe(true);
	});

	it('does not mark an expired, a revoked, an automation or a differently labelled token', () => {
		expect(liveBrowserSession(token({ expires_at: '2026-10-26T11:59:59Z' }), now)).toBe(false);
		expect(liveBrowserSession(token({ revoked_at: '2026-10-20T00:00:00Z' }), now)).toBe(false);
		expect(liveBrowserSession(token({ kind: 'automation' }), now)).toBe(false);
		expect(liveBrowserSession(token({ label: 'cli' }), now)).toBe(false);
	});
});
