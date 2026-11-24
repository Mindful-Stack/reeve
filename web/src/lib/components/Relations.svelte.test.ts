// @vitest-environment jsdom
// Relations.svelte: blockers first under their own label, item links grouped by side, web links by title with their
// host; adding and removing both kinds in the forward form; the informational line; errors beside the control.
import { createRawSnippet, flushSync, mount, unmount, type ComponentProps } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ItemType } from '#lib/api/types.ts';
import { itemLink, searchHit, webLink } from '#lib/test/fixtures.ts';
import { failure, settle, type Call } from '#lib/test/net.ts';
import Relations from './Relations.svelte';

// A story drawer over the board: a linked story opens over the same view.
vi.mock('$app/state', async () => ({
	page: (await import('#lib/test/net.ts')).fakePage({ url: '/board?story=ABC-10' }).page
}));

const net = vi.hoisted(() => ({
	calls: [] as Call[],
	/** What `GET /search` answers. */
	hits: [] as unknown[],
	/** What the next writes answer; a test that expects a write sets it. */
	answer: (c: Call): unknown => {
		throw new Error(`unexpected ${c.method} ${c.path}`);
	}
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch((c) => {
		net.calls.push(c);
		if (c.method === 'GET' && c.path === '/api/v1/search') return net.hits;
		if (c.method === 'GET') throw new Error(`unexpected GET ${c.path}`);
		return net.answer(c);
	});
	return orig();
});
const noWrite = net.answer;

/** The component's own props, not a second declaration of them: a prop the component gains, renames or retypes is
 *  checked against every `render(…)` below by `npm run check`. */
type Props = ComponentProps<typeof Relations>;

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.calls.length = 0;
	net.hits = [];
	net.answer = noWrite;
});

/** Mounts the component as story ABC-10's. `props` is reactive: assigning a field is the host passing a new value. */
function render(over: Partial<Props> = {}) {
	const changed = vi.fn();
	const props = $state<Props>({ itemKey: 'ABC-10', level: 'story', links: [], webLinks: [], onchanged: changed, ...over });
	app = mount(Relations, { target: document.body, props });
	flushSync();
	return { props, changed };
}

const to = (key: string, level: ItemType = 'story', title = `title of ${key}`) => ({ key, level, title });
/** The section whose heading is `name`. */
const section = (name: string) => [...document.body.querySelectorAll('section')].find((s) => s.querySelector('h2, h3')?.textContent === name) as HTMLElement;
/** The groups of the Relations section: each label with where its links open. */
const groups = () =>
	[...section('Relations').querySelectorAll('.group')].map((g) => [g.querySelector('.label')!.textContent, [...g.querySelectorAll('a')].map((a) => a.getAttribute('href'))]);
/** A button by its accessible name: its `aria-label`, else its text. */
const button = (root: ParentNode, name: string) => [...root.querySelectorAll('button')].find((b) => (b.getAttribute('aria-label') ?? b.textContent?.trim()) === name) as HTMLButtonElement;
const form = (name: string) => document.body.querySelector<HTMLFormElement>(`form[aria-label="${name}"]`);
const alertIn = (root: ParentNode | null | undefined) => root?.querySelector('[role="alert"]')?.textContent ?? '';
/** The informational line (the Picker has a status region of its own, hence the class). */
const note = () => section('Relations').querySelector('.informational')!.textContent!.replace(/\s+/g, ' ').trim();
/** The standing line under the item link groups. */
const standing = () => section('Relations').querySelector('.standing') as HTMLElement;
/** A promise the fake server can be made to wait for, so a test can act while a write is out. */
function gate() {
	let release!: () => void;
	const opened = new Promise<void>((r) => (release = r));
	return { opened, release };
}
/** Every write sent, in order. */
const writes = () => net.calls.filter((c) => c.method !== 'GET').map((c) => ({ method: c.method, path: c.path, body: c.body }));
const fill = (input: HTMLInputElement, text: string) => {
	input.focus();
	input.value = text;
	input.dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
};
/** Submits as a person can: only through an enabled submit button. */
const submit = (f: HTMLFormElement) => {
	expect(f.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(false);
	f.requestSubmit();
};
/** Presses the section's add button and waits for its form. */
async function openForm(sectionName: string, add: string, name: string): Promise<HTMLFormElement> {
	button(section(sectionName), add).click();
	return vi.waitFor(() => {
		flushSync();
		const f = form(name);
		expect(f).not.toBeNull();
		return f!;
	});
}
/** Types into the item search and clicks the hit that starts with `key`. */
async function pick(text: string, key: string) {
	fill(form('Add item link')!.querySelector<HTMLInputElement>('input[role="combobox"]')!, text);
	const option = await vi.waitFor(() => {
		flushSync();
		const o = [...document.body.querySelectorAll<HTMLElement>('[role="option"]')].find((x) => x.textContent!.trim().startsWith(`${key} `));
		expect(o).toBeDefined();
		return o!;
	});
	option.click();
	flushSync();
}

describe('Relations: what it lists', () => {
	it('lists blockers first under "blocked by", adding nothing to what the host rendered, then the item links grouped by side', () => {
		const blockers = createRawSnippet(() => ({ render: () => '<ul><li>ABC-6</li></ul>' }));
		render({
			blockers,
			links: [
				itemLink({ type: 'follows', direction: 'out', label: 'follows', item: to('ABC-11') }),
				itemLink({ type: 'follows', direction: 'in', label: 'is followed by', item: to('ABC-2', 'epic') }),
				itemLink({ type: 'follows', direction: 'out', label: 'follows', item: to('ABC-1', 'initiative') })
			]
		});
		expect(section('Relations').querySelector('h2')?.textContent).toBe('Relations');
		expect(groups()).toEqual([
			['blocked by', []],
			['follows', ['/board?story=ABC-11', '/initiatives/ABC-1']],
			['is followed by', ['/epics/ABC-2']]
		]);
		// The label and what the host passed, nothing else: no word here on whether ABC-6 holds the story back.
		const blocked = section('Relations').querySelector('.group')!;
		expect([...blocked.children].map((c) => c.textContent!.trim())).toEqual(['blocked by', 'ABC-6']);
		// A linked item reads as its level (for a screen reader), its key and its title.
		const first = section('Relations').querySelector('a[href="/board?story=ABC-11"]')!;
		expect(first.textContent!.replace(/\s+/g, ' ')).toContain('story ABC-11 title of ABC-11');
		expect(first.getAttribute('data-sveltekit-reset')).toBe('false');
	});

	it('has no "blocked by" group without blockers from the host, and says so when there is nothing', () => {
		render({ level: 'epic', itemKey: 'ABC-5' });
		expect(groups()).toEqual([]);
		// Each empty state also says what the thing is, the first in the words of the item's level.
		expect(section('Relations').querySelector('.empty')!.textContent).toBe('No item links yet. An item link says that this epic follows, duplicates or relates to another item.');
		expect(section('Web links').querySelector('.empty')!.textContent).toBe('No web links yet. A web link points at a page outside the planner.');
		expect(document.body.querySelector('.error')).toBeNull();
	});

	it('always says that item links are informational, in the words of its level, as plain text and not as a status region', () => {
		const cases: [ItemType, string][] = [
			['story', 'Item links are informational. Only a blocker holds a story back.'],
			['epic', 'Item links are informational. They never hold work back.'],
			['initiative', 'Item links are informational. They never hold work back.']
		];
		for (const [level, words] of cases) {
			// With no item link and with one: the line does not depend on the list.
			for (const links of [[], [itemLink({ item: to('ABC-11') })]]) {
				render({ level, links });
				expect(standing().textContent, `${level}, ${links.length} links`).toBe(words);
				expect(standing().getAttribute('role')).toBeNull();
				expect(standing().closest('[role="status"], [role="alert"], [aria-live]')).toBeNull();
				expect(standing().classList.contains('muted')).toBe(true);
				// Under the groups: after the last group in the section.
				const last = [...section('Relations').querySelectorAll('.group')].at(-1);
				if (last) expect(last.compareDocumentPosition(standing()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
				unmount(app!);
				app = null;
				document.body.innerHTML = '';
			}
		}
	});

	it('still says there are no item links when only blockers are listed', () => {
		render({ blockers: createRawSnippet(() => ({ render: () => '<p>Nothing blocks this story.</p>' })) });
		expect(groups()).toEqual([['blocked by', []]]);
		expect(section('Relations').textContent).toContain('No item links yet.');
	});

	it('lists a web link by its title with the host beside it, opening in a new tab without an opener', () => {
		render({
			heading: 'h3',
			webLinks: [webLink({ id: 4, title: 'Scenario page', url: 'https://example.com:8443/wiki/scenario' }), webLink({ id: 5, title: 'Odd', url: 'javascript:alert(1)' })]
		});
		expect(section('Web links').querySelector('h3')?.textContent).toBe('Web links');
		const rows = [...section('Web links').querySelectorAll('li')];
		const a = rows[0].querySelector('a')!;
		expect([a.getAttribute('href'), a.target, a.rel]).toEqual(['https://example.com:8443/wiki/scenario', '_blank', 'noopener noreferrer']);
		expect(a.textContent).toBe('Scenario page (opens in a new tab)');
		expect(rows[0].querySelector('.host')!.textContent).toBe('example.com:8443');
		// A stored URL that is not http(s) never becomes a live link.
		expect(rows[1].querySelector('a')).toBeNull();
		expect(rows[1].textContent).toContain('Odd');
		expect(section('Web links').textContent).not.toContain('No web links yet.');
	});

	it('follows the host: a new list replaces the one shown', () => {
		const { props } = render({ links: [itemLink({ item: to('ABC-11') })] });
		expect(groups()).toEqual([['follows', ['/board?story=ABC-11']]]);
		props.links = [itemLink({ type: 'duplicates', direction: 'in', label: 'is duplicated by', item: to('ABC-12') })];
		props.webLinks = [webLink({ id: 7, title: 'Runbook' })];
		flushSync();
		expect(groups()).toEqual([['is duplicated by', ['/board?story=ABC-12']]]);
		expect(section('Web links').querySelector('a')!.textContent).toContain('Runbook');
	});
});
