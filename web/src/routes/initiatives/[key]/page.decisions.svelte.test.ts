// @vitest-environment jsdom
// The initiative page's Decisions heading counts blocking and accepted decisions (zero parts hidden), and the accepted
// fold starts open only when nothing blocks.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Initiative, RequiredDecision } from '#lib/api/types.ts';
import { decision } from '#lib/test/fixtures.ts';
import type { FakePage } from '#lib/test/net.ts';
import Page from './+page.svelte';

const route = vi.hoisted(() => ({ fake: null as null | FakePage }));
vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	route.fake = fakePage({ url: '/initiatives/ABC-1', params: { key: 'ABC-1' } });
	return { page: route.fake.page };
});

const net = vi.hoisted(() => ({ decisions: [] as RequiredDecision[] }));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(async ({ method, path }) => {
		if (method === 'GET' && (path === '/api/v1/initiatives/ABC-1' || path === '/api/v1/initiatives/ABC-2')) {
			const ini: Initiative = { key: path.slice(-5), title: 'Orion', brief: '', horizon: 'Now', version: 3, target_month: null, owner: null, urgent_mark: null, urgent_sources: [] };
			return ini;
		}
		if (method === 'GET' && (path === '/api/v1/epics' || path === '/api/v1/stories' || path === '/api/v1/activity')) return [];
		if (method === 'GET' && path === '/api/v1/decisions') return net.decisions;
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.decisions = [];
	route.fake!.reset();
});

const accepted = (id: number) => decision({ id, on: 'ABC-1', on_type: 'initiative', check: { kind: 'accepted', commit: 'abc1234def' } });
async function open() {
	app = mount(Page, { target: document.body });
	await vi.waitFor(() => {
		flushSync();
		expect(document.querySelector('#decisions-h')?.textContent).toMatch(/Decisions\s*·/);
	});
}
const fold = () => document.querySelector('#decisions .fold button') as HTMLButtonElement;

describe('decisions heading and accepted fold', () => {
	it('counts only accepted and starts the fold open when nothing blocks', async () => {
		net.decisions = [accepted(1)];
		await open();
		expect(document.querySelector('#decisions-h')!.textContent!.replace(/\s+/g, ' ').trim()).toBe('Decisions · 1 accepted');
		expect(fold().textContent).toBe('1 accepted decision');
		expect(fold().getAttribute('aria-expanded')).toBe('true');
		expect(document.querySelector('#decisions-rest')!.textContent).toContain('decision 1');
	});

	it("forgets the user's choice when the page moves to another initiative", async () => {
		net.decisions = [accepted(1)];
		await open();
		fold().click();
		flushSync();
		expect(fold().getAttribute('aria-expanded')).toBe('false');
		route.fake!.params.set('key', 'ABC-2');
		await vi.waitFor(() => {
			flushSync();
			expect(fold().getAttribute('aria-expanded')).toBe('true');
		});
	});

	it('counts blocking and accepted, with the fold closed', async () => {
		net.decisions = [decision({ id: 2, on: 'ABC-1', on_type: 'initiative' }), accepted(1)];
		await open();
		expect(document.querySelector('#decisions-h')!.textContent!.replace(/\s+/g, ' ').trim()).toBe('Decisions · 1 blocking · 1 accepted');
		expect(fold().getAttribute('aria-expanded')).toBe('false');
		fold().click();
		flushSync();
		expect(fold().getAttribute('aria-expanded')).toBe('true');
	});

	it('opens a decision in the drawer from its title, over this page', async () => {
		net.decisions = [decision({ id: 2, on: 'ABC-1', on_type: 'initiative', title: 'Pick a queue' })];
		await open();
		const title = document.querySelector<HTMLAnchorElement>('#decisions a.title')!;
		expect(title.textContent).toBe('Pick a queue');
		expect(title.getAttribute('href')).toBe('/initiatives/ABC-1?decision=2');
	});
});
