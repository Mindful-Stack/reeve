import { afterEach, describe, expect, expectTypeOf, it, vi } from 'vitest';
import type { components } from './schema';

// openapi-fetch captures globalThis.fetch when the client is created, so the stub goes in before the import.
const fetchStub = vi.fn<typeof fetch>();
vi.stubGlobal('fetch', fetchStub);
const { api, ApiError, Conflict, Refused, unwrap } = await import('./client');
type StoryWithReadiness = components['schemas']['StoryWithReadiness'];

// Fakes are the server's envelope shapes as the generated schema types them, so a wire change breaks this file too.
type Env = components['schemas']['Response_Vec_StoryWithReadiness'];
type Ok = Extract<Env, { outcome: 'ok' }>;
type RefusedEnv = Extract<Env, { outcome: 'refused' }>;
type ErrorEnv = Extract<Env, { outcome: 'error' }>;

const story: StoryWithReadiness = {
	story: {
		key: 'RV-3',
		title: 'Show the board',
		description: '',
		kind: 'story',
		epic: 'RV-2',
		delivery: 'todo',
		owner: null,
		labels: [],
		criteria: [{ text: 'columns render', done: false }],
		blocked_by: [],
		prs: [],
		goals: [],
		urgent_mark: null,
		urgent_sources: [],
		promise: null,
		rank: 1,
		version: 2
	},
	readiness: { ready: true, reasons: [] }
};

function reply(status: number, body: Env) {
	const response = new Response(JSON.stringify(body), { status });
	return Promise.resolve(
		status < 400 ? { data: body, response } : { error: body as ErrorEnv | RefusedEnv, response }
	) as Promise<{ data?: Env; error?: unknown; response: Response }>;
}

afterEach(() => {
	vi.unstubAllGlobals();
	// The client created at import keeps its own reference to fetchStub, so unstubbing the global is safe.
	fetchStub.mockReset();
});

describe('unwrap', () => {
	it('infers the ok value from the path with no annotation', () => {
		const call = () => unwrap(api.GET('/api/v1/stories'));
		expectTypeOf<Awaited<ReturnType<typeof call>>>().toEqualTypeOf<StoryWithReadiness[]>();
	});

	it('returns the value of an ok envelope', async () => {
		const ok: Ok = { outcome: 'ok', value: [story] };
		await expect(unwrap(reply(200, ok))).resolves.toEqual([story]);
	});

	it('throws Refused with the reasons, by body even at 200', async () => {
		const refused: RefusedEnv = { outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: true };
		const e = await unwrap(reply(200, refused)).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(Refused);
		expect(e).not.toBeInstanceOf(ApiError);
		expect((e as InstanceType<typeof Refused>).reasons).toEqual([{ reason: 'no_criteria' }]);
		expect((e as InstanceType<typeof Refused>).overridable).toBe(true);
	});

	it('carries the overridable flag, false when an older server leaves it out', async () => {
		const rule: RefusedEnv = { outcome: 'refused', reasons: [{ reason: 'resolution_required' }], overridable: false };
		const e = await unwrap(reply(422, rule)).catch((x: unknown) => x);
		expect((e as InstanceType<typeof Refused>).overridable).toBe(false);
		const older = { outcome: 'refused', reasons: [{ reason: 'no_criteria' }] } as unknown as RefusedEnv;
		const o = await unwrap(reply(422, older)).catch((x: unknown) => x);
		expect((o as InstanceType<typeof Refused>).overridable).toBe(false);
	});

	it('throws Refused for a 422 refusal', async () => {
		const refused: RefusedEnv = { outcome: 'refused', reasons: [{ reason: 'untriaged' }], overridable: true };
		await expect(unwrap(reply(422, refused))).rejects.toBeInstanceOf(Refused);
	});

	it('throws Conflict carrying current on a 409', async () => {
		const conflict: ErrorEnv = { outcome: 'error', code: 'conflict', message: 'version conflict: current is 7', current: 7 };
		const e = await unwrap(reply(409, conflict)).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(Conflict);
		expect((e as InstanceType<typeof Conflict>).current).toBe(7);
	});

	it('throws Conflict(-1) when the server omits current', async () => {
		const conflict: ErrorEnv = { outcome: 'error', code: 'conflict', message: 'concurrent update, retry' };
		const e = await unwrap(reply(409, conflict)).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(Conflict);
		expect((e as InstanceType<typeof Conflict>).current).toBe(-1);
	});

	it('throws ApiError for any other error code', async () => {
		const bad: ErrorEnv = { outcome: 'error', code: 'validation', message: 'title is empty' };
		const e = await unwrap(reply(400, bad)).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e).not.toBeInstanceOf(Conflict);
		expect(e).toMatchObject({ status: 400, code: 'validation', message: 'title is empty' });
	});

	it('sends a 401 to sign-in with a full page load keeping next, and never settles', async () => {
		const assign = vi.fn();
		vi.stubGlobal('location', { pathname: '/board', search: '?epic=RV-2', assign });
		const unauth: ErrorEnv = { outcome: 'error', code: 'unauthenticated', message: 'unauthenticated' };
		let settled = false;
		void unwrap(reply(401, unauth)).finally(() => (settled = true));
		await new Promise((r) => setTimeout(r, 10));
		expect(assign).toHaveBeenCalledWith('/auth/signin?next=%2Fboard%3Fepic%3DRV-2');
		expect(settled).toBe(false);
	});

	it('turns a rejected request into ApiError(transport)', async () => {
		const e = await unwrap(Promise.reject(new TypeError('fetch failed'))).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e).toMatchObject({ code: 'transport' });
	});
});

// These go through the real openapi-fetch client with a stubbed fetch, so the request it builds and the data/error split
// it applies are what is asserted, not hand-built results.
describe('api through openapi-fetch', () => {
	const baseUrl = 'http://reeve.test';
	const json = (status: number, body: unknown) =>
		new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
	const sent = (): Request => fetchStub.mock.calls[0][0] as Request;

	it('sends the csrf header and same-origin credentials on a GET', async () => {
		fetchStub.mockResolvedValue(json(200, { outcome: 'ok', value: [] } satisfies Ok));
		await unwrap(api.GET('/api/v1/stories', { baseUrl }));
		expect(sent().method).toBe('GET');
		expect(sent().headers.get('X-Reeve-Client')).toBe('web');
		expect(sent().credentials).toBe('same-origin');
	});

	it('sends the csrf header and same-origin credentials on a POST', async () => {
		fetchStub.mockResolvedValue(json(200, { outcome: 'ok', value: null }));
		await api.POST('/api/v1/stories/{key}/move', { baseUrl, params: { path: { key: 'RV-3' } } } as never);
		expect(sent().method).toBe('POST');
		expect(sent().headers.get('X-Reeve-Client')).toBe('web');
		expect(sent().credentials).toBe('same-origin');
	});

	it('turns a 422 json refusal into Refused with its reasons', async () => {
		const refused: RefusedEnv = { outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: true };
		fetchStub.mockResolvedValue(json(422, refused));
		const e = await unwrap(api.GET('/api/v1/stories', { baseUrl })).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(Refused);
		expect((e as InstanceType<typeof Refused>).reasons).toEqual([{ reason: 'no_criteria' }]);
	});

	it('turns a 502 html body into ApiError(transport)', async () => {
		fetchStub.mockResolvedValue(new Response('<html>Bad Gateway</html>', { status: 502, headers: { 'content-type': 'text/html' } }));
		const e = await unwrap(api.GET('/api/v1/stories', { baseUrl })).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e).toMatchObject({ code: 'transport', status: 502 });
	});

	it('turns a 200 non-json body into ApiError(transport)', async () => {
		fetchStub.mockResolvedValue(new Response('<html>hello</html>', { status: 200, headers: { 'content-type': 'text/html' } }));
		const e = await unwrap(api.GET('/api/v1/stories', { baseUrl })).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e).toMatchObject({ code: 'transport' });
	});

	it('turns a network failure into ApiError(transport)', async () => {
		fetchStub.mockRejectedValue(new TypeError('fetch failed'));
		const e = await unwrap(api.GET('/api/v1/stories', { baseUrl })).catch((x: unknown) => x);
		expect(e).toBeInstanceOf(ApiError);
		expect(e).toMatchObject({ code: 'transport' });
	});
});
