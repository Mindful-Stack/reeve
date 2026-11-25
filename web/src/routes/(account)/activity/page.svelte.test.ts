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
				activity({ id: 1, entity_key: 'ABC-9', action: 'attachment_added', payload: { attachment_id: 'x', filename: 'shot.png' } }),
				activity({ id: 0, entity_type: 'brief_template', entity_key: '3', action: 'brief_template_created', payload: { level: 'initiative', name: 'Scenario' } })
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
			expect(document.body.querySelectorAll('.action')).toHaveLength(3);
		});
		expect([...document.body.querySelectorAll('.action')].map((e) => e.textContent)).toEqual([
			'promised by 20 Nov 2026 to Acme',
			'added attachment shot.png',
			'created the initiative brief template Scenario'
		]);
	});

	it("links a brief template's entry to the templates page, without the template's id", async () => {
		app = mount(Page, { target: document.body });
		await vi.waitFor(() => {
			flushSync();
			expect(document.body.querySelectorAll('.entity')).toHaveLength(3);
		});
		const words = (e: Element) => e.textContent!.replace(/\s+/g, ' ').trim();
		const [story, , template] = [...document.body.querySelectorAll('.entity')];
		// The control: an item's entry names its type and its key, which is the link.
		expect(words(story)).toBe('story ABC-12');
		expect(story.querySelector('a')!.textContent).toBe('ABC-12');
		// A template has no key a person knows it by: its name is in the action's words, and the link is the page.
		expect(words(template)).toBe('Templates');
		expect(template.querySelector('a')!.getAttribute('href')).toBe('/templates');
	});
});
