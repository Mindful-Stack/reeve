import { describe, expect, it, vi } from 'vitest';
import type { Call } from './net';

const seen = vi.hoisted(() => ({ calls: [] as Call[] }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('./net');
	installFetch((c) => {
		seen.calls.push(c);
		return { ok: true };
	});
	return orig();
});
const { api } = await import('#lib/api/client.ts');

describe('the fetch fake', () => {
	it('records a binary body byte for byte and does not parse it as JSON', async () => {
		await api.POST('/api/v1/attachments', {
			params: { query: { filename: 'a.png' } },
			body: new Uint8Array([1, 2, 3]) as never,
			bodySerializer: (b: unknown) => b as BodyInit,
			headers: { 'Content-Type': 'application/octet-stream' }
		});
		const c = seen.calls.at(-1)!;
		expect([...c.bytes!]).toEqual([1, 2, 3]);
		expect(c.body).toBeNull();
		expect(c.headers.get('content-type')).toBe('application/octet-stream');
		expect(c.headers.get('x-reeve-client')).toBe('web');
	});

	it('still parses a JSON write', async () => {
		await api.POST('/api/v1/items/{key}/comments', { params: { path: { key: 'GRA-1' } }, body: { body: 'hi' } });
		expect(seen.calls.at(-1)!.body).toEqual({ body: 'hi' });
	});
});
