// @vitest-environment jsdom
// The account activity page words each entry through `actionLabel`, without the key (the row shows it beside the action):
// a promise from its payload, an attachment by its file.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { activity } from '#lib/test/fixtures.ts';
import type { Call } from '#lib/test/net.ts';
import Page from './+page.svelte';

vi.mock('$app/state', async () => {
	const { SvelteURL } = await import('svelte/reactivity');
	return { page: { url: new SvelteURL('http://localhost/activity') } };
});
vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('#lib/events.svelte.ts', async (orig) => ({
	...(await orig<typeof import('#lib/events.svelte.ts')>()),
	onLive: () => {}
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch((c: Call) => {
		if (c.method === 'GET' && c.path === '/api/v1/activity')
			return [
				activity({ id: 2, entity_key: 'ABC-12', action: 'promised', payload: { item_type: 'story', on: '2026-11-20', to: 'Acme', previous: null } }),
				activity({ id: 1, entity_key: 'ABC-9', action: 'attachment_added', payload: { attachment_id: 'x', filename: 'shot.png' } })
			];
		throw new Error(`unexpected ${c.method} ${c.path}`);
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('account activity', () => {
	it('words a promise and an attachment through actionLabel', async () => {
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelectorAll('.action')).toHaveLength(2);
		});
		expect([...document.body.querySelectorAll('.action')].map((e) => e.textContent)).toEqual([
			'promised by 20 Nov 2026 to Acme',
			'added attachment shot.png'
		]);
	});
});
