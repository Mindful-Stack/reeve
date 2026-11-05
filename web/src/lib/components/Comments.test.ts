// @vitest-environment jsdom
// A comment body is Markdown, rendered and sanitised like a story's description (#lib/markdown.ts).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Comment } from '#lib/api/types.ts';
import { comment as commentDto } from '#lib/test/fixtures.ts';
import Comments from './Comments.svelte';

const net = vi.hoisted(() => ({ sent: [] as string[] }));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(({ method, path }) => {
		net.sent.push(`${method} ${path}`);
		return [];
	});
	return orig();
});

function comment(body: string): Comment {
	return commentDto({ id: 1, author: 'dan@example.com', at: '2026-10-01T00:00:00Z', body });
}

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

function render(body: string): HTMLElement {
	app = mount(Comments, {
		target: document.body,
		props: { itemKey: 'RV-1', comments: [comment(body)], onposted: () => {} }
	});
	flushSync();
	return document.body.querySelector('.thread') as HTMLElement;
}

describe('Comments body', () => {
	it('renders an imported Markdown link as an anchor', () => {
		const a = render('[attachment](https://x)').querySelector('a');
		expect(a?.getAttribute('href')).toBe('https://x');
		expect(a?.textContent).toBe('attachment');
	});

	it('renders no script from a script body', () => {
		const thread = render('hi <script>alert(1)</script>');
		expect(thread.querySelector('script')).toBeNull();
		expect(thread.innerHTML).not.toContain('alert');
	});
});

describe('Comments composer', () => {
	it('posts to the item by its key, whatever level it is', async () => {
		let posted = false;
		app = mount(Comments, {
			target: document.body,
			props: { itemKey: 'ABC-6', comments: [], onposted: () => (posted = true) }
		});
		flushSync();
		const area = document.body.querySelector('textarea')!;
		area.value = 'Should Orion go first?';
		area.dispatchEvent(new Event('input'));
		document.body.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true }));
		await vi.waitFor(() => expect(posted).toBe(true));
		expect(net.sent).toEqual(['POST /api/v1/items/ABC-6/comments']);
		expect(area.getAttribute('aria-label')).toBe('comment on ABC-6');
	});
});
