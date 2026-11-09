import { api, unwrap } from '#lib/api/client.ts';
import { onStream } from '#lib/events.svelte.ts';
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

let generation = 0;

/** Fetches `/me` again and replaces `session.me`, so the role follows the server without a reload (kb/ADR-0015 R5).
 *  A 401 leaves through `unwrap`, as on load. An answer to an older call never overwrites a newer one. */
export async function refresh(): Promise<void> {
	const mine = ++generation;
	const me = await unwrap(api.GET('/api/v1/me'));
	if (mine === generation) session.me = me;
}

const RETRY_MS = 5000;
const ACCESS_ACTIONS = new Set(['access_allowed', 'access_removed', 'admin_granted', 'admin_revoked']);

/** Refreshes the signed-in user on every access entry, whoever its subject (removing a domain can change the viewer's
 *  access without naming them), and on every reopen of the stream. Returns the unsubscribe. */
export function followAccess(): () => void {
	let retry: ReturnType<typeof setTimeout> | undefined;
	// A failed refresh is retried once after RETRY_MS, so a transient error does not leave a stale role until some later
	// trigger. One retry per failure, and the retry schedules none: this is not polling (kb/ADR-0012 R3). Any newer
	// refresh cancels a pending retry.
	const attempt = (again: boolean) => {
		clearTimeout(retry);
		retry = undefined;
		refresh().catch(() => {
			if (again) retry = setTimeout(() => attempt(false), RETRY_MS);
		});
	};
	const quietly = () => attempt(true);
	const stop = onStream({
		entry: (e) => {
			if (ACCESS_ACTIONS.has(e.action)) quietly();
		},
		reopened: quietly
	});
	quietly(); // the stream starts at the newest id and does not replay what happened since `load()`
	return () => {
		clearTimeout(retry);
		stop();
	};
}
