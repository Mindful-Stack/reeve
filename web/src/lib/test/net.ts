// Shared network and route fakes for component tests.
//
// openapi-fetch reads `fetch` and `Request` once, when #lib/api/client.ts creates the client, so `installFetch` must run
// before any import of the client: call it inside `vi.mock('#lib/api/client.ts', async (orig) => { installFetch(…); return orig(); })`
// (import this module with `await import` inside the factory), with a sync `vi.hoisted` holder for what the handler records.
import { SvelteMap, SvelteURL } from 'svelte/reactivity';

/** One request as the fake server sees it: the method, the absolute URL, its path and the parsed JSON body of a write
 *  (null for a read or an empty body). */
export type Call = { method: string; url: URL; path: string; body: unknown };

/** Answers a call: a `Response` is sent as is, anything else is the value of an `ok` envelope. Throwing rejects the
 *  fetch, as a network failure does. */
export type Handler = (call: Call) => unknown | Promise<unknown>;

const ORIGIN = 'http://localhost';

export const json = (body: unknown, status = 200) =>
	new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

/** The `ok` envelope around `value`. */
export const ok = (value: unknown) => json({ outcome: 'ok', value });

/** An `error` envelope; `internal` with status 500 unless named. */
export const failure = (message: string, status = 500, code = 'internal') => json({ outcome: 'error', code, message }, status);

/** Installs the fake `Request` (Node's needs an absolute URL; the client sends paths relative to the page) and a `fetch`
 *  that hands every request to `handler`. */
export function installFetch(handler: Handler): void {
	const Base = globalThis.Request;
	globalThis.Request = class extends Base {
		constructor(input: RequestInfo | URL, init?: RequestInit) {
			super(typeof input === 'string' ? new URL(input, ORIGIN) : input, init);
		}
	};
	globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
		const req = input instanceof Request ? input : new Request(String(input), init);
		const url = new URL(req.url, ORIGIN);
		let body: unknown = null;
		if (req.method !== 'GET' && req.method !== 'HEAD') {
			const text = await req.text();
			body = text ? JSON.parse(text) : null;
		}
		const answer = await handler({ method: req.method, url, path: url.pathname, body });
		return answer instanceof Response ? answer : ok(answer);
	};
}

/** Lets everything already queued behind a promise run: `rounds` turns of the microtask queue (no timer, so no clock
 *  to outwait), for a test that proves something did NOT happen. Pair it with a count of the requests sent and
 *  `flushSync()`; a positive expectation uses `vi.waitFor` instead. */
export async function settle(rounds = 100): Promise<void> {
	for (let i = 0; i < rounds; i++) await Promise.resolve();
}

/** A stand-in for `$app/state`'s `page` with a reactive URL and params, and a `goto` that writes the URL as SvelteKit's
 *  navigation would, so the page re-renders from it. Mock `$app/navigation` with `goto` (wrapped in `vi.fn` to assert
 *  on it) and `$app/state` with `{ page }`; `params` sets a route parameter, as a navigation to another item does. */
export function fakePage({ url, params = {} }: { url: string; params?: Record<string, string> }) {
	const start = new URL(url, ORIGIN).href;
	const href = new SvelteURL(start);
	const values = new SvelteMap(Object.entries(params));
	const routeParams: Record<string, string | undefined> = {};
	for (const name of Object.keys(params)) Object.defineProperty(routeParams, name, { enumerable: true, get: () => values.get(name) });
	const page = { url: href, params: routeParams };
	return {
		page,
		params: values,
		goto: async (to: string | URL, _opts?: unknown) => {
			href.href = new URL(String(to), href).href;
		},
		/** Back to the URL and params it started with (for `afterEach`). */
		reset: () => {
			href.href = start;
			for (const [name, value] of Object.entries(params)) values.set(name, value);
		}
	};
}

export type FakePage = ReturnType<typeof fakePage>;
