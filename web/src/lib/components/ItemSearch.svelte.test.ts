// @vitest-environment jsdom
// ItemSearch: asks GET /search for what is typed and offers the hits through the Picker, unnarrowed; says what it is
// doing while there is nothing to offer; never shows an answer a newer text overtook; drops the pick when something
// else is typed; leaves the item itself out and says when it shows only the first eight.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { searchHit } from '#lib/test/fixtures.ts';
import { settle } from '#lib/test/net.ts';

const net = vi.hoisted(() => ({
	asked: [] as string[],
	answers: new Map<string, unknown[]>(),
	/** A text whose answer waits for its gate. */
	gates: new Map<string, Promise<void>>(),
	fail: false
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	installFetch(async (c) => {
		if (c.method !== 'GET' || c.path !== '/api/v1/search') throw new Error(`unexpected ${c.method} ${c.path}`);
		const q = c.url.searchParams.get('q') ?? '';
		net.asked.push(q);
		await net.gates.get(q);
		return net.fail ? failure('database down') : (net.answers.get(q) ?? []);
	});
	return orig();
});
const { default: Harness } = await import('./ItemSearchHarness.test.svelte');

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.asked.length = 0;
	net.answers.clear();
	net.gates.clear();
	net.fail = false;
});

function setup() {
	app = mount(Harness, { target: document.body });
	flushSync();
	const input = document.body.querySelector<HTMLInputElement>('input[role="combobox"]')!;
	/** Replaces the whole text at once, as typing over the selected label does. */
	const type = (text: string) => {
		input.focus();
		input.value = text;
		input.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
	};
	const options = () => [...document.body.querySelectorAll<HTMLElement>('[role="option"]')];
	const offered = () => options().map((o) => o.textContent!.trim());
	const status = () => document.body.querySelector('[role="status"]')!.textContent;
	const picked = () => document.body.querySelector('[data-testid="picked"]')!.textContent;
	return { input, type, options, offered, status, picked };
}

describe('ItemSearch', () => {
	it('asks the server for the typed text and offers every hit it answers, also one whose title does not hold the text', async () => {
		net.answers.set('pay', [
			searchHit({ key: 'ABC-7', kind: 'bug', title: 'Billing account', matched: ['description'] }),
			searchHit({ key: 'ABC-2', item: 'epic', kind: null, title: 'Payments' })
		]);
		const { input, type, options, offered, picked } = setup();
		expect(input.getAttribute('aria-label')).toBe('Other item');
		type(' pay ');
		await vi.waitFor(() => {
			flushSync();
			expect(offered()).toEqual(['ABC-7 Billing account · bug', 'ABC-2 Payments · epic']);
		});
		expect(net.asked).toEqual(['pay']);
		options()[1].click();
		await vi.waitFor(() => {
			flushSync();
			expect(input.value).toBe('ABC-2 Payments · epic');
		});
		expect(picked()).toBe('ABC-2 epic');
	});

	it('drops the pick as soon as something else is typed: the item picked before is neither bound nor offered', async () => {
		net.answers.set('alpha', [searchHit({ key: 'ABC-3', title: 'Alpha' })]);
		const { input, type, options, offered, status, picked } = setup();
		type('alpha');
		await vi.waitFor(() => {
			flushSync();
			expect(offered()).toEqual(['ABC-3 Alpha · story']);
		});
		options()[0].click();
		await vi.waitFor(() => {
			flushSync();
			expect(picked()).toBe('ABC-3 story');
		});
		// "beta" matches nothing. From the first keystroke on, while it is searched for and after the answer, ABC-3 is
		// neither bound nor offered.
		type('beta');
		expect(picked()).toBe('');
		expect(offered()).toEqual([]);
		await vi.waitFor(() => {
			flushSync();
			expect(status()).toBe('No item found');
		});
		expect(offered()).toEqual([]);
		expect(picked()).toBe('');
		// Left without a pick, the box is empty: it does not go back to showing the item that is no longer picked.
		input.blur();
		await vi.waitFor(() => {
			flushSync();
			expect(input.value).toBe('');
		});
	});

	it('leaves the item itself out of the hits, offers eight of what is left and says so only when there are more', async () => {
		const others = (n: number) => Array.from({ length: n }, (_, i) => searchHit({ key: `ABC-${i + 20}` }));
		// The harness is item ABC-1's form. Ten hits: ABC-1 itself, named exactly and so first, and nine others.
		net.answers.set('bill', [searchHit({ key: 'ABC-1', exact: true }), ...others(9)]);
		// Nine hits: ABC-1 and eight others. With the item itself left out, all of them are shown.
		net.answers.set('bil', [searchHit({ key: 'ABC-1', exact: true }), ...others(8)]);
		const { type, options, status } = setup();
		const keys = () => options().map((o) => o.textContent!.trim().split(' ')[0]);
		const eight = ['ABC-20', 'ABC-21', 'ABC-22', 'ABC-23', 'ABC-24', 'ABC-25', 'ABC-26', 'ABC-27'];
		type('bill');
		await vi.waitFor(() => {
			flushSync();
			expect(keys()).toEqual(eight);
		});
		expect(status()).toBe('Showing the first 8. Type more to narrow.');
		// On screen the same words close the list (the Picker draws them from this attribute).
		expect(document.body.querySelector('[role="listbox"]')!.getAttribute('data-more')).toBe('Showing the first 8. Type more to narrow.');
		type('bil');
		await vi.waitFor(() => {
			flushSync();
			expect(keys()).toEqual(eight);
			expect(status()).toBe('');
		});
		expect(document.body.querySelector('[role="listbox"]')!.hasAttribute('data-more')).toBe(false);
	});

	it('says what to type, then that it is searching, then that nothing was found, and drops an answer a newer text overtook', async () => {
		let release!: () => void;
		net.gates.set('a', new Promise<void>((r) => (release = r)));
		net.answers.set('a', [searchHit({ key: 'ABC-3', title: 'Alpha' })]);
		const { input, type, offered, status } = setup();
		input.focus();
		input.click();
		flushSync();
		expect(status()).toBe('Type a key or part of a title');
		type('a');
		expect(status()).toBe('Searching…');
		await vi.waitFor(() => expect(net.asked).toEqual(['a']));
		type('ab');
		await vi.waitFor(() => {
			flushSync();
			expect(status()).toBe('No item found');
		});
		release();
		await settle();
		await new Promise((r) => setTimeout(r, 0));
		await settle();
		flushSync();
		expect(offered()).toEqual([]);
		expect(status()).toBe('No item found');
	});

	it('says so when the search fails', async () => {
		net.fail = true;
		const { type, status } = setup();
		type('pay');
		await vi.waitFor(() => {
			flushSync();
			expect(status()).toBe('Could not search: database down');
		});
	});
});
