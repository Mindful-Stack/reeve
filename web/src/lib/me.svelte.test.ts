import { beforeEach, describe, expect, it, vi } from 'vitest';

const get = vi.fn();
vi.mock('#lib/api/client.ts', () => ({ api: { GET: (path: string) => get(path) }, unwrap: (p: Promise<unknown>) => p }));
const { load, session } = await import('./me.svelte.ts');

const me = { email: 'dan@example.com', role: 'admin', token_kind: 'interactive' };
const config = { horizon_buckets: ['now'] };

beforeEach(() => get.mockReset());

describe('load', () => {
	it('retries after a failure instead of caching it, then shares the success', async () => {
		get.mockImplementation(async (path: string) => (path === '/api/v1/me' ? me : config));
		get.mockRejectedValueOnce(new Error('boom'));
		await expect(load()).rejects.toThrow('boom');
		await load();
		expect(session.me).toEqual(me);
		expect(session.config).toEqual(config);
		const calls = get.mock.calls.length;
		await load();
		expect(get.mock.calls.length).toBe(calls);
	});
});
