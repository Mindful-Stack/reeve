// Pure helpers for the token page.
import type { Token } from '#lib/api/types.ts';

/** The label the server gives the interactive token a browser sign-in issues (reeve_identity::SESSION_LABEL). */
export const SESSION_LABEL = 'web session';

/** A LIVE browser session: an interactive token labelled as a sign-in, neither revoked nor expired at `now`. The API
 *  does not say which token this request used, so every live one is marked, the current session among them. */
export function liveBrowserSession(t: Token, now: Date): boolean {
	if (t.kind !== 'interactive' || t.label !== SESSION_LABEL || t.revoked_at) return false;
	return !t.expires_at || new Date(t.expires_at).getTime() > now.getTime();
}
