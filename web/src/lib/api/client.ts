import createClient from 'openapi-fetch';
import type { paths } from './schema';
import type { Reason } from './types';

/** The gate said no; `reasons` is what the UI shows, and `overridable` is the server's word on whether an interactive
 *  override can get past them. Never folded into `ApiError`: a refusal is not a failure. */
export class Refused extends Error {
	constructor(
		public reasons: Reason[],
		public overridable: boolean
	) {
		super('refused');
	}
}
export class ApiError extends Error {
	constructor(
		public status: number,
		public code: string,
		message: string
	) {
		super(message);
	}
}
/** A stale version. `current` is the row's version now, or -1 when the server retried a serialisation failure. */
export class Conflict extends ApiError {
	constructor(public current: number) {
		super(409, 'conflict', 'someone else changed this; reloaded');
	}
}

/** Every request carries the CSRF header; cookie-authenticated non-GET requests without it are rejected server-side. */
const raw = createClient<paths>({
	baseUrl: '',
	headers: { 'X-Reeve-Client': 'web' },
	credentials: 'same-origin'
});

type Envelope =
	| { outcome: 'ok'; value: unknown }
	| { outcome: 'refused'; reasons: Reason[]; overridable?: boolean }
	| { outcome: 'error'; code: string; message: string; current?: number | null };

type OkValue<D> = D extends { outcome: 'ok'; value: infer V } ? V : never;

/** Unwraps the envelope by its body (`outcome`, then `code`), never by HTTP status. 401 sends the browser to sign in with
 *  a full page load (which also drops every cached module, including `session`) and never resolves for this caller.
 *  Anything thrown below the envelope (a network failure, a non-JSON 2xx body) becomes `ApiError('transport')`, so callers
 *  only ever handle `Refused | Conflict | ApiError`. The value type is inferred from openapi-fetch's `data`, so a caller cannot name a
 *  shape the path does not return. */
export async function unwrap<D extends { outcome: string }>(
	p: Promise<{ data?: D; error?: unknown; response: Response }>
): Promise<OkValue<D>> {
	let result: Awaited<typeof p>;
	try {
		result = await p;
	} catch (e) {
		throw new ApiError(0, 'transport', e instanceof Error ? e.message : 'request failed');
	}
	const { data, error, response } = result;
	if (response.status === 401) {
		location.assign(`/auth/signin?next=${encodeURIComponent(location.pathname + location.search)}`);
		return new Promise<never>(() => {});
	}
	const env = (data ?? error) as Envelope | undefined;
	if (!env || typeof env !== 'object' || !('outcome' in env)) {
		throw new ApiError(response.status, 'transport', response.statusText || 'unexpected response');
	}
	if (env.outcome === 'ok') return env.value as OkValue<D>;
	if (env.outcome === 'refused') throw new Refused(env.reasons, env.overridable === true);
	if (env.code === 'conflict') throw new Conflict(env.current ?? -1);
	throw new ApiError(response.status, env.code, env.message);
}

/** Callers write `unwrap(api.GET('/api/v1/stories', { params: { query: { ready: true } } }))`. */
export const api = raw;
