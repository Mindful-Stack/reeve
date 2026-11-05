// @vitest-environment jsdom
// The "New story" form: a double submit sends one POST with one key, "none" sends no epic, a fixed epic is sent with no
// picker, and a failure shows its reason and keeps the draft and the key for the retry.
import { flushSync, mount, unmount, type ComponentProps } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Story } from '#lib/api/types.ts';
import { epic } from '#lib/test/fixtures.ts';
import StoryForm from './StoryForm.svelte';

type Posted = { idempotency_key: string; title: string; kind: Story['kind']; epic: string | null; criteria: unknown[]; labels: unknown[] };

const net = vi.hoisted(() => ({ posts: [] as Posted[], state: { failNext: 0 } }));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	const { story } = await import('#lib/test/fixtures.ts');
	installFetch(async ({ method, path, body }) => {
		if (method !== 'POST' || path !== '/api/v1/stories') throw new Error(`unexpected ${method} ${path}`);
		const posted = body as Posted;
		net.posts.push(posted);
		await new Promise((r) => setTimeout(r, 5));
		if (net.state.failNext > 0) {
			net.state.failNext--;
			return failure('boom');
		}
		return story({ key: 'SMK-9', title: posted.title, kind: posted.kind, epic: posted.epic });
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.posts.length = 0;
	net.state.failNext = 0;
});

const form = () => document.body.querySelector('form[aria-label="New story"]') as HTMLFormElement;
function field(label: string): HTMLInputElement {
	const l = [...form().querySelectorAll('label')].find((x) => x.textContent?.trim().startsWith(label));
	return l?.querySelector('input,select,textarea') as HTMLInputElement;
}
function type(el: HTMLInputElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	el.dispatchEvent(new Event('change', { bubbles: true }));
	flushSync();
}
function mountForm(props: Omit<ComponentProps<typeof StoryForm>, 'oncreated'> = {}) {
	const oncreated = vi.fn(async () => {});
	app = mount(StoryForm, { target: document.body, props: { oncreated, ...props } });
	flushSync();
	return oncreated;
}
const epics = [epic({ key: 'ABC-1', title: 'One' }), epic({ key: 'ABC-2', title: 'Two' })];

describe('StoryForm', () => {
	it('sends one request with one key for a double submit', async () => {
		const oncreated = mountForm({ epics });
		type(field('Title'), 'A story');
		form().requestSubmit();
		form().requestSubmit();
		await new Promise((r) => setTimeout(r, 0));
		flushSync();
		expect(field('Title').disabled).toBe(true);
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts).toHaveLength(1);
		expect(net.posts[0].idempotency_key).toEqual(expect.any(String));
	});

	it('sends epic null when the epic is "none", and the picked epic otherwise', async () => {
		const oncreated = mountForm({ epics });
		expect(field('Epic').value).toBe('');
		type(field('Title'), 'Loose');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[0]).toEqual({ idempotency_key: expect.any(String), title: 'Loose', kind: 'story', epic: null, criteria: [], labels: [] });
		type(field('Epic'), 'ABC-2');
		type(field('Title'), 'Placed');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(2));
		expect(net.posts[1].epic).toBe('ABC-2');
		expect(net.posts[1].idempotency_key).not.toBe(net.posts[0].idempotency_key);
	});

	it('sends the fixed epic and shows no picker on an epic page', async () => {
		const oncreated = mountForm({ epic: 'ABC-5' });
		expect(field('Epic')).toBeUndefined();
		type(field('Title'), 'Child');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[0].epic).toBe('ABC-5');
		expect(form().querySelector('button[type="submit"]')!.textContent).toBe('Create story');
	});

	it('shows a failure as an alert and keeps the draft and the key for the retry', async () => {
		const oncreated = mountForm({ epics });
		type(field('Title'), 'A story');
		net.state.failNext = 1;
		form().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(form().querySelector('[role="alert"]')?.textContent).toContain('boom');
		});
		expect(field('Title').value).toBe('A story');
		expect(oncreated).not.toHaveBeenCalled();
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[1].idempotency_key).toBe(net.posts[0].idempotency_key);
	});
});
