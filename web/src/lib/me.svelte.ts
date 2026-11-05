import { api, unwrap } from '#lib/api/client.ts';
import type { InstanceConfig, Me } from '#lib/api/types.ts';

/** The signed-in user and the instance config. A mutable rune object: fields are assigned, the binding never is. */
export const session = $state<{ me: Me | null; config: InstanceConfig | null }>({ me: null, config: null });

let pending: Promise<void> | null = null;

/** Fills `session` once; later calls share the same promise. Any failure clears it so the next call retries (a 401 never
 *  settles, but it leaves through a full page load, which resets this module). */
export function load(): Promise<void> {
	pending ??= Promise.all([unwrap(api.GET('/api/v1/me')), unwrap(api.GET('/api/v1/config'))]).then(
		([me, config]) => {
			session.me = me;
			session.config = config;
		},
		(e: unknown) => {
			pending = null;
			throw e;
		}
	);
	return pending;
}

export function isAdmin(): boolean {
	return session.me?.role === 'admin';
}

/** A person at a browser or CLI login, as opposed to an automation token. */
export function isInteractive(): boolean {
	return session.me?.token_kind === 'interactive';
}
