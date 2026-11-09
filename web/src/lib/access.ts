// What an access removal would do, computed from the Access page's own data: the server decides, this only words the
// inline confirmation and hides actions the server would refuse.
import type { Access, AccessEntry, User } from '#lib/api/types.ts';

export type Removal = { kind: 'domain' | 'email'; value: string };

/** Trim and lowercase ASCII only, as the server does. */
export const norm = (s: string) => s.trim().replace(/[A-Z]/g, (c) => c.toLowerCase());
const domainOf = (email: string): string | null => (email.includes('@') ? email.slice(email.lastIndexOf('@') + 1) : null);
const keep = (rows: AccessEntry[], kind: Removal['kind'], skip: Removal | null) =>
	rows.filter((r) => !(skip && skip.kind === kind && r.source === 'database' && r.value === skip.value));

/** The entry that allows `email` once `skip` is gone: the email itself, `@domain`, or null. */
export function allowedBy(a: Access, email: string, skip: Removal | null = null): string | null {
	const e = norm(email);
	if (keep(a.emails, 'email', skip).some((r) => r.value === e)) return e;
	const d = domainOf(e);
	if (d !== null && keep(a.domains, 'domain', skip).some((r) => r.value === d)) return `@${d}`;
	return null;
}

function isAdminAfter(a: Access, email: string, skip: Removal | null): boolean {
	const e = norm(email);
	return a.admins.some((r) => r.value === e && (r.source === 'config' || allowedBy(a, email, skip) !== null));
}

/** Whether removing `skip` would leave `me` unallowed or no longer admin (the server refuses it too). */
export function locksOut(a: Access, me: string, kind: Removal['kind'], value: string): boolean {
	const skip = { kind, value };
	return allowedBy(a, me, skip) === null || !isAdminAfter(a, me, skip);
}

/** The inline confirmation for removing a database entry. */
export function removalNote(a: Access, users: User[], kind: Removal['kind'], value: string): string {
	const skip = { kind, value };
	const lost = (email: string) => allowedBy(a, email) !== null && allowedBy(a, email, skip) === null;
	const demoted = a.admins.filter((r) => r.source === 'database' && r.allowed && lost(r.value)).map((r) => r.value);
	const demotes = demoted.length ? ` ${demoted.join(', ')} ${demoted.length === 1 ? 'loses' : 'lose'} admin.` : '';
	if (kind === 'email') {
		const still = allowedBy(a, value, skip);
		return still
			? `Remove ${value}? They stay allowed through ${still}.${demotes}`
			: `Remove ${value}? They lose access on their next request.${demotes}`;
	}
	const n = users.filter((u) => lost(u.email)).length;
	return `Remove @${value}? ${n} listed ${n === 1 ? 'user loses' : 'users lose'} access on their next request.${demotes}`;
}
