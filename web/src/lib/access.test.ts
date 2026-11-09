import { describe, expect, it } from 'vitest';
import type { Access, AccessEntry, User } from '#lib/api/types.ts';
import { allowedBy, locksOut, removalNote } from './access.ts';

const row = (value: string, source: 'config' | 'database', allowed = true): AccessEntry =>
	({ value, source, also_in_config: false, allowed, added_by: source === 'database' ? 'dan@example.com' : null, added_at: null }) as AccessEntry;
const access: Access = {
	domains: [row('example.com', 'config'), row('partner.org', 'database')],
	emails: [row('pat@partner.org', 'database'), row('guest@other.org', 'database')],
	admins: [row('dan@example.com', 'config'), row('pat@partner.org', 'database'), row('ana@partner.org', 'database')]
};
const users = ['dan@example.com', 'pat@partner.org', 'ana@partner.org', 'kim@partner.org'].map((email) => ({ email }) as User);

describe('allowedBy', () => {
	it('names what still allows an email, leaving out the row being removed', () => {
		expect(allowedBy(access, 'pat@partner.org')).toBe('pat@partner.org');
		expect(allowedBy(access, 'pat@partner.org', { kind: 'email', value: 'pat@partner.org' })).toBe('@partner.org');
		expect(allowedBy(access, 'guest@other.org', { kind: 'email', value: 'guest@other.org' })).toBeNull();
	});
});

describe('removalNote', () => {
	it('says when nothing else allows them', () => {
		expect(removalNote(access, users, 'email', 'guest@other.org')).toBe('Remove guest@other.org? They lose access on their next request.');
	});
	it('says what still allows them', () => {
		expect(removalNote(access, users, 'email', 'pat@partner.org')).toBe('Remove pat@partner.org? They stay allowed through @partner.org.');
	});
	it('counts the listed users a domain cuts off and names the admins it demotes', () => {
		// pat stays allowed by email; ana and kim are cut off; ana's admin grant goes
		expect(removalNote(access, users, 'domain', 'partner.org')).toBe(
			'Remove @partner.org? 2 listed users lose access on their next request. ana@partner.org loses admin.'
		);
	});
});

describe('mixed case, whitespace and config duplicates', () => {
	it('normalises input to allowedBy', () => {
		expect(allowedBy(access, '  PAT@Partner.ORG ')).toBe('pat@partner.org');
		expect(allowedBy(access, 'Kim@PARTNER.org')).toBe('@partner.org');
		expect(allowedBy(access, 'no-at-sign')).toBeNull();
	});
	it('words a database duplicate of a config entry as still allowed through it', () => {
		const dup: Access = { ...access, domains: [row('partner.org', 'config'), row('partner.org', 'database')] };
		expect(removalNote(dup, users, 'domain', 'partner.org')).toBe('Remove @partner.org? 0 listed users lose access on their next request.');
		const email: Access = { ...access, emails: [row('guest@other.org', 'config'), row('guest@other.org', 'database')] };
		expect(removalNote(email, users, 'email', 'guest@other.org')).toBe('Remove guest@other.org? They stay allowed through guest@other.org.');
	});
	it('locks out a config admin allowed only by the database row being removed', () => {
		const a: Access = { domains: [], emails: [row('boss@other.org', 'database')], admins: [row('boss@other.org', 'config')] };
		expect(locksOut(a, 'boss@other.org', 'email', 'boss@other.org')).toBe(true);
	});
});

describe('locksOut', () => {
	it('is true when the removal leaves me unallowed or no longer admin', () => {
		expect(locksOut(access, 'pat@partner.org', 'email', 'pat@partner.org')).toBe(false); // the domain still allows
		expect(locksOut(access, 'ana@partner.org', 'domain', 'partner.org')).toBe(true);
		expect(locksOut(access, 'dan@example.com', 'domain', 'partner.org')).toBe(false);
	});
});
