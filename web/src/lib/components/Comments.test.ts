// @vitest-environment jsdom
// A comment body is Markdown, rendered and sanitised like a story's description (#lib/markdown.ts).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Comment } from '#lib/api/types.ts';
import { comment as commentDto } from '#lib/test/fixtures.ts';
import Comments from './Comments.svelte';

const net = vi.hoisted(() => ({ sent: [] as string[], bodies: [] as unknown[], hold: null as Promise<void> | null }));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(async ({ method, path, body }) => {
		net.sent.push(`${method} ${path}`);
		net.bodies.push(body);
		if (path === '/api/v1/attachments') {
			if (net.hold) await net.hold;
			return { id: 'img1', url: '/api/v1/attachments/img1', filename: 'shot.png', content_type: 'image/png', byte_size: 4, uploaded_by: 'dan@example.com', uploaded_at: '2026-11-10T09:00:00Z' };
		}
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

describe('Comments images', () => {
	it('blocks posting while an image uploads, then sends it and empties the row', async () => {
		const { file } = await import('#lib/test/events.ts');
		let release!: () => void;
		net.hold = new Promise((r) => (release = r));
		net.sent = [];
		net.bodies = [];
		app = mount(Comments, { target: document.body, props: { itemKey: 'ABC-6', comments: [], onposted: () => {} } });
		flushSync();
		const input = document.body.querySelector<HTMLInputElement>('input[type=file]')!;
		Object.defineProperty(input, 'files', { value: [file('shot.png')], configurable: true });
		input.dispatchEvent(new Event('change', { bubbles: true }));
		flushSync();
		const button = document.body.querySelector<HTMLButtonElement>('button[type=submit]')!;
		await vi.waitFor(() => expect(button.textContent).toBe('Uploading…'));
		expect(button.disabled).toBe(true);
		release();
		await vi.waitFor(() => expect(document.body.querySelector('textarea')!.value).toBe('![shot](/api/v1/attachments/img1)'));
		flushSync();
		expect(button.disabled).toBe(false);
		document.body.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true }));
		await vi.waitFor(() => expect(net.sent).toContain('POST /api/v1/items/ABC-6/comments'));
		expect(net.bodies.at(-1)).toMatchObject({ body: '![shot](/api/v1/attachments/img1)', attachments: ['img1'] });
		await vi.waitFor(() => expect(document.body.querySelector('.uploads li')).toBeNull());
		net.hold = null;
	});

	it('keeps Add image from submitting the form', () => {
		app = mount(Comments, { target: document.body, props: { itemKey: 'ABC-6', comments: [], onposted: () => {} } });
		flushSync();
		const add = [...document.body.querySelectorAll('button')].find((b) => b.textContent === 'Add image')!;
		expect(add.type).toBe('button');
	});
});
