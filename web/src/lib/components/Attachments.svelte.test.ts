// @vitest-environment jsdom
// The Attachments disclosure: hidden with none, a confirm before a delete that is for good, focus after it, and the
// window event that tells rendered images (#lib/rendered.ts).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Call } from '#lib/test/net.ts';

const net = vi.hoisted(() => ({ calls: [] as Call[], list: [] as unknown[], failGet: false, failDelete: false }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	installFetch((c) => {
		net.calls.push(c);
		if (c.method === 'GET' && c.path === '/api/v1/items/ABC-1/attachments') {
			if (net.failGet) return failure('database down', 500);
			return net.list;
		}
		if (c.method === 'DELETE') return net.failDelete ? failure('database down', 500) : {};
		throw new Error(`unexpected ${c.method} ${c.path}`);
	});
	return orig();
});
const { default: Attachments } = await import('./Attachments.svelte');
const { default: Harness } = await import('./AttachmentsHarness.test.svelte');

const dto = (id: string, filename: string) => ({
	id,
	url: `/api/v1/attachments/${id}`,
	filename,
	content_type: 'image/png',
	byte_size: 10,
	uploaded_by: 'dan@example.com',
	uploaded_at: '2026-11-10T09:00:00Z',
	owner: { item: 'story', key: 'ABC-1' }
});

let app: ReturnType<typeof mount> | null = null;
beforeEach(() => {
	net.calls = [];
	net.failGet = false;
	net.failDelete = false;
});
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	vi.restoreAllMocks();
});

async function render() {
	app = mount(Attachments, { target: document.body, props: { itemKey: 'ABC-1' } });
	await vi.waitFor(() => expect(net.calls.some((c) => c.method === 'GET')).toBe(true));
	await new Promise((r) => setTimeout(r, 0));
	flushSync();
}

describe('Attachments', () => {
	it('renders nothing for an item without any', async () => {
		net.list = [];
		await render();
		expect(document.body.querySelector('details')).toBeNull();
	});

	it('lists them collapsed, and deletes only after the confirm, then tells rendered images and moves focus', async () => {
		net.list = [dto('a1', 'one.png'), dto('a2', 'two.png')];
		await render();
		const details = document.body.querySelector('details')!;
		expect(details.open).toBe(false);
		details.open = true;
		expect(details.querySelector('summary')!.textContent).toBe('Attachments · 2');
		const del = details.querySelectorAll<HTMLButtonElement>('button.delete');
		expect(del[0].getAttribute('aria-label')).toBe('Delete attachment one.png');

		const ask = vi.spyOn(window, 'confirm').mockReturnValue(false);
		del[0].click();
		expect(ask).toHaveBeenCalledWith('Delete one.png? It disappears wherever it is shown, and cannot be restored.');
		expect(net.calls.some((c) => c.method === 'DELETE')).toBe(false);

		ask.mockReturnValue(true);
		const heard: string[] = [];
		const listen = (e: Event) => heard.push((e as CustomEvent<{ id: string }>).detail.id);
		window.addEventListener('reeve:attachment-removed', listen);
		del[0].click();
		await vi.waitFor(() => expect(heard).toEqual(['a1']));
		window.removeEventListener('reeve:attachment-removed', listen);
		expect(net.calls.find((c) => c.method === 'DELETE')!.path).toBe('/api/v1/attachments/a1');
		flushSync();
		await vi.waitFor(() => expect(document.activeElement?.getAttribute('aria-label')).toBe('Delete attachment two.png'));
		expect(document.body.querySelector('summary')!.textContent).toBe('Attachments · 1');
	});

	it('gives focus to the section heading when the last one is deleted, and shows a failed load with Retry', async () => {
		net.list = [dto('a1', 'one.png')];
		const section = document.createElement('section');
		section.innerHTML = '<h3>Description</h3>';
		document.body.append(section);
		app = mount(Attachments, { target: section, props: { itemKey: 'ABC-1' } });
		await vi.waitFor(() => expect(section.querySelector('details')).not.toBeNull());
		section.querySelector('details')!.open = true;
		vi.spyOn(window, 'confirm').mockReturnValue(true);
		net.list = [];
		section.querySelector<HTMLButtonElement>('button.delete')!.click();
		await vi.waitFor(() => expect(document.activeElement?.tagName).toBe('H3'));
		expect(section.querySelector('details')).toBeNull();
	});

	it('shows a failed load in a Notice whose Retry loads again', async () => {
		net.failGet = true;
		net.list = [dto('a1', 'one.png')];
		app = mount(Attachments, { target: document.body, props: { itemKey: 'ABC-1' } });
		await vi.waitFor(() => expect(document.body.textContent).toContain('Could not load attachments: database down'));
		net.failGet = false;
		[...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Retry')!.click();
		await vi.waitFor(() => expect(document.body.querySelector('summary')?.textContent).toBe('Attachments · 1'));
		expect(document.body.textContent).not.toContain('Could not load');
	});

	it('keeps the row and says so when a delete fails, and tells no image', async () => {
		net.list = [dto('a1', 'one.png')];
		await render();
		await vi.waitFor(() => expect(document.body.querySelector('details')).not.toBeNull());
		document.body.querySelector('details')!.open = true;
		vi.spyOn(window, 'confirm').mockReturnValue(true);
		net.failDelete = true;
		const heard: string[] = [];
		const listen = (e: Event) => heard.push((e as CustomEvent<{ id: string }>).detail.id);
		window.addEventListener('reeve:attachment-removed', listen);
		document.body.querySelector<HTMLButtonElement>('button.delete')!.click();
		await vi.waitFor(() => expect(document.body.querySelector('[role=alert]')?.textContent).toContain('Could not delete one.png'));
		window.removeEventListener('reeve:attachment-removed', listen);
		expect(heard).toEqual([]);
		expect(document.body.querySelectorAll('button.delete')).toHaveLength(1);
	});

	it('stays open and loads once when its item is refetched with the same key', async () => {
		net.list = [dto('a1', 'one.png')];
		const props = $state({ story: { key: 'ABC-1' } });
		app = mount(Harness, { target: document.body, props });
		await vi.waitFor(() => expect(document.body.querySelector('details')).not.toBeNull());
		document.body.querySelector('details')!.open = true;
		const gets = net.calls.filter((c) => c.method === 'GET').length;
		props.story = { key: 'ABC-1' };
		flushSync();
		await new Promise((r) => setTimeout(r, 10));
		expect(document.body.querySelector('details')?.open).toBe(true);
		expect(net.calls.filter((c) => c.method === 'GET').length).toBe(gets);
	});
});
