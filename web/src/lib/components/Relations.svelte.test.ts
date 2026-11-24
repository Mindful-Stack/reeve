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
			['story', 'Item links are informational and never hold a story back. Of the relations listed here, only a blocker can.'],
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

	it('in the story drawer names the blockers list by its label and sets the item links apart under "Item links"; an epic has no sub-label', () => {
		// The host's snippet is handed the id of the "blocked by" label, for its list to be named by.
		const blockers = createRawSnippet<[string]>((label) => ({ render: () => `<ul aria-labelledby="${label()}"><li>ABC-6</li></ul>` }));
		const after = (a: Element, b: Element) => Boolean(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING);
		const sub = () => section('Relations').querySelector('.sublabel');
		render({ blockers, links: [itemLink({ item: to('ABC-11') })] });
		const list = section('Relations').querySelector('ul[aria-labelledby]')!;
		expect(list.textContent).toBe('ABC-6');
		expect(document.getElementById(list.getAttribute('aria-labelledby')!)!.textContent).toBe('blocked by');
		// Between the blockers and the first item link group, and not a group itself.
		expect(sub()!.textContent).toBe('Item links');
		const [blocked, follows] = [...section('Relations').querySelectorAll('.group')];
		expect(after(blocked, sub()!) && after(sub()!, follows)).toBe(true);
		expect(sub()!.closest('.group')).toBeNull();
		unmount(app!);
		document.body.innerHTML = '';
		// With no item link it stands above the empty state.
		render({ blockers });
		expect(after(sub()!, section('Relations').querySelector('.empty')!)).toBe(true);
		unmount(app!);
		document.body.innerHTML = '';
		// An epic or an initiative has no blockers: nothing to tell its item links from.
		for (const level of ['epic', 'initiative'] as const) {
			for (const links of [[], [itemLink({ item: to('ABC-11') })]]) {
				render({ level, links });
				expect(sub(), `${level}, ${links.length} links`).toBeNull();
				unmount(app!);
				document.body.innerHTML = '';
			}
		}
		app = null;
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

describe('Relations: item links', () => {
	it('adds an item link in the forward form, lists the answer, closes the form onto its button and says a follows between two stories is informational', async () => {
		net.hits = [searchHit({ key: 'ABC-11', title: 'Billing account', exact: true })];
		const { changed } = render();
		const f = await openForm('Relations', '+ Add item link', 'Add item link');
		const select = f.querySelector('select')!;
		await vi.waitFor(() => expect(document.activeElement).toBe(select));
		expect(select.getAttribute('aria-label')).toBe('Item link name');
		expect([...select.options].map((o) => [o.value, o.textContent])).toEqual([
			['follows', 'follows'],
			['duplicates', 'duplicates'],
			['relates_to', 'relates to']
		]);
		// The form reads as a sentence about this item ("This story follows …"), never as a bare key, and the select is
		// described by the hint for the name chosen.
		expect(f.querySelector('.subject')!.textContent).toBe('This story');
		const hint = f.querySelector('.hint')!;
		expect(hint.textContent).toBe('This story comes after the item you pick. For the reverse, open the other item and add the item link there.');
		expect(select.getAttribute('aria-describedby')).toBe(hint.id);
		// On a row of its own above the controls, where the open search list does not cover it.
		expect(f.firstElementChild).toBe(hint);
		// The search field says what it is for while it is empty; its name stays "Other item".
		const search = f.querySelector<HTMLInputElement>('input[role="combobox"]')!;
		expect([search.getAttribute('aria-label'), search.placeholder]).toEqual(['Other item', 'Search for an item']);
		// Nothing picked yet: there is nothing to add.
		expect(f.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
		await pick('ABC-11', 'ABC-11');
		net.answer = () => [itemLink({ type: 'follows', direction: 'out', label: 'follows', item: to('ABC-11', 'story', 'Billing account') })];
		submit(f);
		await vi.waitFor(() => {
			flushSync();
			expect(groups()).toEqual([['follows', ['/board?story=ABC-11']]]);
		});
		expect(writes()).toEqual([{ method: 'POST', path: '/api/v1/items/ABC-10/links', body: { type: 'follows', to: 'ABC-11' } }]);
		expect(note()).toBe('Informational: this does not hold ABC-10 back. To hold it back, add ABC-11 with "+ Add blocker" above.');
		// A sentence about the control above it, not a command to copy: no code element.
		expect(section('Relations').querySelector('.informational code')).toBeNull();
		expect(form('Add item link')).toBeNull();
		await vi.waitFor(() => expect(document.activeElement?.textContent?.trim()).toBe('+ Add item link'));
		expect(changed).toHaveBeenCalledTimes(1);
		expect(section('Relations').textContent).not.toContain('No item links yet.');
	});

	it('says nothing after any other name, and what the host passes next replaces the list a write showed', async () => {
		net.hits = [searchHit({ key: 'ABC-11' })];
		const { props } = render();
		const f = await openForm('Relations', '+ Add item link', 'Add item link');
		const select = f.querySelector('select')!;
		const choose = (name: string) => {
			select.value = name;
			select.dispatchEvent(new Event('change', { bubbles: true }));
			flushSync();
		};
		// The hint follows the name: none for relates_to (and then the select points at none), its own for duplicates.
		choose('relates_to');
		expect(f.querySelector('.hint')).toBeNull();
		expect(select.hasAttribute('aria-describedby')).toBe(false);
		choose('duplicates');
		expect(f.querySelector('.hint')!.textContent).toBe('This story covers the same work as the item you pick.');
		await pick('ABC-11', 'ABC-11');
		net.answer = () => [itemLink({ type: 'duplicates', direction: 'out', label: 'duplicates', item: to('ABC-11') })];
		submit(f);
		await vi.waitFor(() => {
			flushSync();
			expect(groups()).toEqual([['duplicates', ['/board?story=ABC-11']]]);
		});
		expect(writes()).toEqual([{ method: 'POST', path: '/api/v1/items/ABC-10/links', body: { type: 'duplicates', to: 'ABC-11' } }]);
		expect(note()).toBe('');
		// The host's refetch, with a link someone else added meanwhile.
		props.links = [itemLink({ type: 'duplicates', direction: 'out', label: 'duplicates', item: to('ABC-11') }), itemLink({ type: 'relates_to', direction: 'out', label: 'relates to', item: to('ABC-3', 'epic') })];
		flushSync();
		expect(groups()).toEqual([
			['duplicates', ['/board?story=ABC-11']],
			['relates to', ['/epics/ABC-3']]
		]);
	});

	it('removes an out link by naming this item first, once, and moves focus to the next Remove', async () => {
		const { changed } = render({
			links: [itemLink({ type: 'follows', direction: 'out', label: 'follows', item: to('ABC-11') }), itemLink({ type: 'follows', direction: 'in', label: 'is followed by', item: to('ABC-12') })]
		});
		// The server answers the links of the item named first: ABC-10's, with the one that is left.
		net.answer = () => [itemLink({ type: 'follows', direction: 'in', label: 'is followed by', item: to('ABC-12') })];
		const remove = button(section('Relations'), 'Remove item link: follows ABC-11');
		remove.focus();
		remove.click();
		remove.click();
		await vi.waitFor(() => {
			flushSync();
			expect(groups()).toEqual([['is followed by', ['/board?story=ABC-12']]]);
		});
		expect(writes()).toEqual([{ method: 'DELETE', path: '/api/v1/items/ABC-10/links/follows/ABC-11', body: null }]);
		await vi.waitFor(() => expect(document.activeElement?.getAttribute('aria-label')).toBe('Remove item link: is followed by ABC-12'));
		expect(changed).toHaveBeenCalledTimes(1);
	});

	it('removes an in link with the two keys swapped, and does not take the other item\'s list for its own', async () => {
		render({ links: [itemLink({ type: 'follows', direction: 'in', label: 'is followed by', item: to('ABC-12') })] });
		// DELETE /items/ABC-12/links/follows/ABC-10 answers ABC-12's links: here, one that has nothing to do with ABC-10.
		net.answer = () => [itemLink({ type: 'relates_to', direction: 'out', label: 'relates to', item: to('ABC-99', 'epic') })];
		const remove = button(section('Relations'), 'Remove item link: is followed by ABC-12');
		remove.focus();
		remove.click();
		await vi.waitFor(() => {
			flushSync();
			expect(section('Relations').textContent).toContain('No item links yet.');
		});
		expect(writes()).toEqual([{ method: 'DELETE', path: '/api/v1/items/ABC-12/links/follows/ABC-10', body: null }]);
		expect(groups()).toEqual([]);
		// The last Remove is gone: focus goes to the section's add button, not to the page.
		await vi.waitFor(() => expect(document.activeElement?.textContent?.trim()).toBe('+ Add item link'));
	});

	it('filters the list on screen when a removal answers late: what the host loaded meanwhile stays, and focus the person moved is left alone', async () => {
		const { props, changed } = render({ links: [itemLink({ type: 'follows', direction: 'in', label: 'is followed by', item: to('ABC-12') })] });
		const g = gate();
		net.answer = async () => {
			await g.opened;
			// ABC-12's links, as the DELETE answers them.
			return [];
		};
		const remove = button(section('Relations'), 'Remove item link: is followed by ABC-12');
		remove.focus();
		remove.click();
		await vi.waitFor(() => expect(writes()).toHaveLength(1));
		// While the DELETE is out, a live event makes the host refetch: someone else added a relation.
		props.links = [
			itemLink({ type: 'follows', direction: 'in', label: 'is followed by', item: to('ABC-12') }),
			itemLink({ type: 'relates_to', direction: 'out', label: 'relates to', item: to('ABC-3', 'epic') })
		];
		flushSync();
		expect(groups()).toEqual([
			['is followed by', ['/board?story=ABC-12']],
			['relates to', ['/epics/ABC-3']]
		]);
		// And the person moves on, to the new row's link, inside this same section.
		const elsewhere = section('Relations').querySelector<HTMLAnchorElement>('a[href="/epics/ABC-3"]')!;
		elsewhere.focus();
		expect(document.activeElement).toBe(elsewhere);
		g.release();
		await vi.waitFor(() => {
			flushSync();
			expect(groups()).toEqual([['relates to', ['/epics/ABC-3']]]);
		});
		await vi.waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
		await settle();
		flushSync();
		expect(document.activeElement).toBe(elsewhere);
	});

	it('keeps the list the host passed while an add was out: the late answer does not hide what someone else added', async () => {
		net.hits = [searchHit({ key: 'ABC-11' })];
		const { props, changed } = render();
		const f = await openForm('Relations', '+ Add item link', 'Add item link');
		await pick('ABC-11', 'ABC-11');
		const mine = itemLink({ type: 'follows', direction: 'out', label: 'follows', item: to('ABC-11') });
		const theirs = itemLink({ type: 'relates_to', direction: 'out', label: 'relates to', item: to('ABC-3', 'epic') });
		const g = gate();
		net.answer = async () => {
			await g.opened;
			// The list as it was when the server wrote the link: before the other person's.
			return [mine];
		};
		submit(f);
		await vi.waitFor(() => expect(writes()).toHaveLength(1));
		// While the answer is out, a live event makes the host refetch: both links are there by now.
		props.links = [mine, theirs];
		flushSync();
		g.release();
		// The host is still asked to refetch, and until it has (or if that read fails) the newer list stays.
		await vi.waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
		await settle();
		flushSync();
		expect(groups()).toEqual([
			['follows', ['/board?story=ABC-11']],
			['relates to', ['/epics/ABC-3']]
		]);
		expect(form('Add item link')).toBeNull();
	});

	it('disables Add again when something else is typed after a pick, and sends nothing for the item picked before', async () => {
		net.hits = [searchHit({ key: 'ABC-11' })];
		render();
		const f = await openForm('Relations', '+ Add item link', 'Add item link');
		const add = f.querySelector<HTMLButtonElement>('button[type="submit"]')!;
		await pick('ABC-11', 'ABC-11');
		expect(add.disabled).toBe(false);
		// Another text, for which the server finds nothing: ABC-11 must not stay picked behind it.
		net.hits = [];
		fill(f.querySelector<HTMLInputElement>('input[role="combobox"]')!, 'zzz');
		expect(add.disabled).toBe(true);
		await vi.waitFor(() => {
			flushSync();
			expect(f.querySelector('[role="status"]')!.textContent).toBe('No item found');
		});
		expect(add.disabled).toBe(true);
		expect(document.body.querySelectorAll('[role="option"]')).toHaveLength(0);
		// Enter in a field submits a form whatever its button says: the handler must hold nothing to send either.
		f.requestSubmit();
		await settle();
		expect(writes()).toEqual([]);
	});

	it('shows a refused item link beside the form, keeps the form and the pick, and names an item the server does not know', async () => {
		// The item itself is among the server's hits and is not offered. The server's refusal of a self link cannot
		// come from this form any more; it is still the server's rule, so it is answered here for another pick and must
		// be shown like any refusal.
		net.hits = [searchHit({ key: 'ABC-10', title: 'This story', exact: true }), searchHit({ key: 'ABC-11' })];
		const { changed } = render();
		const f = await openForm('Relations', '+ Add item link', 'Add item link');
		fill(f.querySelector<HTMLInputElement>('input[role="combobox"]')!, 'ABC-1');
		const offered = await vi.waitFor(() => {
			flushSync();
			const o = [...document.body.querySelectorAll<HTMLElement>('[role="option"]')];
			expect(o.map((x) => x.textContent!.trim().split(' ')[0])).toEqual(['ABC-11']);
			return o;
		});
		offered[0].click();
		flushSync();
		net.answer = () => failure('an item cannot have an item link to itself', 400, 'validation');
		submit(f);
		await vi.waitFor(() => {
			flushSync();
			expect(alertIn(form('Add item link'))).toBe('an item cannot have an item link to itself');
		});
		// Focus is back in the form, not lost to the page.
		await vi.waitFor(() => expect(document.activeElement?.getAttribute('role')).toBe('combobox'));
		net.answer = () => failure('not found', 404, 'not_found');
		submit(form('Add item link')!);
		await vi.waitFor(() => {
			flushSync();
			expect(alertIn(form('Add item link'))).toBe('No item ABC-11.');
		});
		// Both refused writes named this item first and the pick second.
		expect(writes()).toEqual([
			{ method: 'POST', path: '/api/v1/items/ABC-10/links', body: { type: 'follows', to: 'ABC-11' } },
			{ method: 'POST', path: '/api/v1/items/ABC-10/links', body: { type: 'follows', to: 'ABC-11' } }
		]);
		expect(groups()).toEqual([]);
		expect(note()).toBe('');
		expect(changed).not.toHaveBeenCalled();
	});

	it('shows a failed removal beside the list, keeps the link and gives focus back to the button that was pressed, unless the person moved it', async () => {
		const both = [
			['follows', ['/board?story=ABC-11']],
			['relates to', ['/epics/ABC-3']]
		];
		render({ links: [itemLink({ item: to('ABC-11') }), itemLink({ type: 'relates_to', direction: 'out', label: 'relates to', item: to('ABC-3', 'epic') })] });
		const pressed = () => button(section('Relations'), 'Remove item link: follows ABC-11');
		let g = gate();
		net.answer = async () => {
			await g.opened;
			return failure('database down');
		};
		pressed().focus();
		pressed().click();
		// A browser may drop focus to the page when the focused button is disabled. The test drops it itself, at once
		// (the button is still enabled in the DOM until the click's state change is flushed), and checks that it did.
		pressed().blur();
		expect(document.activeElement).toBe(document.body);
		await vi.waitFor(() => expect(writes()).toHaveLength(1));
		g.release();
		await vi.waitFor(() => {
			flushSync();
			expect(alertIn(section('Relations'))).toBe('Could not remove the item link to ABC-11: database down');
		});
		expect(groups()).toEqual(both);
		await vi.waitFor(() => expect(document.activeElement).toBe(pressed()));
		expect(pressed().disabled).toBe(false);

		// Again, and this time the person moves to another row while the request is out: focus stays there.
		g = gate();
		pressed().click();
		await vi.waitFor(() => expect(writes()).toHaveLength(2));
		flushSync();
		expect(pressed().disabled).toBe(true);
		const elsewhere = section('Relations').querySelector<HTMLAnchorElement>('a[href="/epics/ABC-3"]')!;
		elsewhere.focus();
		g.release();
		await vi.waitFor(() => {
			flushSync();
			expect(pressed().disabled).toBe(false);
		});
		await settle();
		flushSync();
		expect(document.activeElement).toBe(elsewhere);
		expect(groups()).toEqual(both);
	});

	it('after a failed removal whose row a reload took meanwhile, focus goes to the next Remove, else to the add button, not to the page', async () => {
		const first = itemLink({ item: to('ABC-11') });
		const second = itemLink({ type: 'relates_to', direction: 'out', label: 'relates to', item: to('ABC-3', 'epic') });
		for (const [left, lands] of [
			[[second], 'Remove item link: relates to ABC-3'],
			[[], '+ Add item link']
		] as const) {
			const { props } = render({ links: [first, second] });
			const g = gate();
			net.answer = async () => {
				await g.opened;
				return failure('database down');
			};
			const pressed = button(section('Relations'), 'Remove item link: follows ABC-11');
			pressed.focus();
			pressed.click();
			await vi.waitFor(() => expect(writes()).toHaveLength(1));
			// A live reload while the DELETE is out: someone else removed the row already, and its button goes with it.
			props.links = [...left];
			flushSync();
			expect(pressed.isConnected).toBe(false);
			expect(document.activeElement).toBe(document.body);
			g.release();
			await vi.waitFor(() => {
				flushSync();
				expect(alertIn(section('Relations'))).toBe('Could not remove the item link to ABC-11: database down');
			});
			await vi.waitFor(() => expect(document.activeElement?.getAttribute('aria-label') ?? document.activeElement?.textContent?.trim()).toBe(lands));
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
			net.calls.length = 0;
		}
	});

	it('Escape in the form cancels it without the key reaching the host', async () => {
		render();
		const f = await openForm('Relations', '+ Add item link', 'Add item link');
		const e = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
		f.querySelector('select')!.dispatchEvent(e);
		await vi.waitFor(() => {
			flushSync();
			expect(form('Add item link')).toBeNull();
		});
		expect(e.defaultPrevented).toBe(true);
		expect(writes()).toEqual([]);
	});
});

describe('Relations: web links', () => {
	const inputs = (f: HTMLFormElement) => [...f.querySelectorAll<HTMLInputElement>('input')];
	/** Where the listed web links point, in order. */
	const hrefs = () => [...section('Web links').querySelectorAll('li a')].map((a) => a.getAttribute('href'));

	it('adds a web link, lists it as the server stored it, and closes the form onto its button', async () => {
		const { changed } = render({ level: 'epic', itemKey: 'ABC-5' });
		const f = await openForm('Web links', '+ Add web link', 'Add web link');
		const [title, url] = inputs(f);
		await vi.waitFor(() => expect(document.activeElement).toBe(title));
		expect(f.querySelector<HTMLButtonElement>('button[type="submit"]')!.disabled).toBe(true);
		// The URL field says what it wants before anything is refused, and is described by that hint.
		const hint = f.querySelector('.hint')!;
		expect(hint.textContent).toBe('The full address, starting with https://');
		expect(url.getAttribute('aria-describedby')).toBe(hint.id);
		expect(title.hasAttribute('aria-describedby')).toBe(false);
		fill(title, ' Scenario page ');
		fill(url, 'HTTPS://Example.com/wiki/scenario');
		// The server trims the title and stores the URL as its parser writes it.
		net.answer = () => [webLink({ id: 4, title: 'Scenario page', url: 'https://example.com/wiki/scenario' })];
		submit(f);
		await vi.waitFor(() => {
			flushSync();
			expect(section('Web links').querySelector('li a')?.getAttribute('href')).toBe('https://example.com/wiki/scenario');
		});
		expect(writes()).toEqual([{ method: 'POST', path: '/api/v1/items/ABC-5/web-links', body: { title: 'Scenario page', url: 'HTTPS://Example.com/wiki/scenario' } }]);
		expect(section('Web links').querySelector('.host')!.textContent).toBe('example.com');
		expect(form('Add web link')).toBeNull();
		await vi.waitFor(() => expect(document.activeElement?.textContent?.trim()).toBe('+ Add web link'));
		expect(changed).toHaveBeenCalledTimes(1);
	});

	it('removes a web link by its id, once, lists the answer and moves focus to the add button, for each of two removals', async () => {
		const { changed } = render({ webLinks: [webLink({ id: 4, title: 'Scenario page' }), webLink({ id: 9, title: 'Runbook', url: 'https://example.com/runbook' })] });
		net.answer = () => [webLink({ id: 9, title: 'Runbook', url: 'https://example.com/runbook' })];
		const remove = button(section('Web links'), 'Remove web link Scenario page');
		remove.focus();
		remove.click();
		remove.click();
		await vi.waitFor(() => {
			flushSync();
			expect([...section('Web links').querySelectorAll('li a')].map((a) => a.getAttribute('href'))).toEqual(['https://example.com/runbook']);
		});
		expect(writes()).toEqual([{ method: 'DELETE', path: '/api/v1/items/ABC-10/web-links/4', body: null }]);
		// To the section's add button, although another Remove is left.
		await vi.waitFor(() => expect(document.activeElement?.textContent?.trim()).toBe('+ Add web link'));
		expect(changed).toHaveBeenCalledTimes(1);

		net.answer = () => [];
		const second = button(section('Web links'), 'Remove web link Runbook');
		second.focus();
		second.click();
		await vi.waitFor(() => {
			flushSync();
			expect(section('Web links').textContent).toContain('No web links yet.');
		});
		// The second removal named the second web link's id: the requests, not only the empty list the test answered.
		expect(writes()).toEqual([
			{ method: 'DELETE', path: '/api/v1/items/ABC-10/web-links/4', body: null },
			{ method: 'DELETE', path: '/api/v1/items/ABC-10/web-links/9', body: null }
		]);
		await vi.waitFor(() => expect(changed).toHaveBeenCalledTimes(2));
		await vi.waitFor(() => expect(document.activeElement?.textContent?.trim()).toBe('+ Add web link'));
	});

	it('leaves focus where the person moved it while a web link removal was out', async () => {
		const { changed } = render({ webLinks: [webLink({ id: 4, title: 'Scenario page' }), webLink({ id: 9, title: 'Runbook', url: 'https://example.com/runbook' })] });
		const g = gate();
		net.answer = async () => {
			await g.opened;
			return [webLink({ id: 9, title: 'Runbook', url: 'https://example.com/runbook' })];
		};
		const remove = button(section('Web links'), 'Remove web link Scenario page');
		remove.focus();
		remove.click();
		await vi.waitFor(() => expect(writes()).toHaveLength(1));
		const elsewhere = section('Web links').querySelector<HTMLAnchorElement>('a[href="https://example.com/runbook"]')!;
		elsewhere.focus();
		expect(document.activeElement).toBe(elsewhere);
		g.release();
		await vi.waitFor(() => {
			flushSync();
			expect(section('Web links').querySelectorAll('li')).toHaveLength(1);
		});
		await vi.waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
		await settle();
		flushSync();
		expect(document.activeElement).toBe(elsewhere);
	});

	it('keeps the list the host passed while a web link add was out: the late answer does not hide what someone else added', async () => {
		const { props, changed } = render();
		const f = await openForm('Web links', '+ Add web link', 'Add web link');
		const [title, url] = inputs(f);
		fill(title, 'Scenario page');
		fill(url, 'https://example.com/wiki/scenario');
		const mine = webLink({ id: 4, title: 'Scenario page', url: 'https://example.com/wiki/scenario' });
		const theirs = webLink({ id: 5, title: 'Runbook', url: 'https://example.com/runbook' });
		const g = gate();
		net.answer = async () => {
			await g.opened;
			return [mine];
		};
		submit(f);
		await vi.waitFor(() => expect(writes()).toHaveLength(1));
		props.webLinks = [mine, theirs];
		flushSync();
		g.release();
		await vi.waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
		await settle();
		flushSync();
		expect(hrefs()).toEqual(['https://example.com/wiki/scenario', 'https://example.com/runbook']);
		expect(form('Add web link')).toBeNull();
	});

	it('keeps the list the host passed while a web link removal was out: the late answer does not hide what someone else added', async () => {
		const gone = webLink({ id: 4, title: 'Scenario page', url: 'https://example.com/wiki/scenario' });
		const theirs = webLink({ id: 5, title: 'Runbook', url: 'https://example.com/runbook' });
		const { props, changed } = render({ webLinks: [gone] });
		const g = gate();
		net.answer = async () => {
			await g.opened;
			// The list as it was when the server removed the web link: before the other person's.
			return [];
		};
		button(section('Web links'), 'Remove web link Scenario page').click();
		await vi.waitFor(() => expect(writes()).toHaveLength(1));
		// The host's read, taken after someone else added a web link and before the removal landed: the row removed
		// here is dropped from it, and the other person's stays.
		props.webLinks = [gone, theirs];
		flushSync();
		g.release();
		await vi.waitFor(() => expect(changed).toHaveBeenCalledTimes(1));
		await settle();
		flushSync();
		expect(hrefs()).toEqual(['https://example.com/runbook']);
	});

	it('shows what the server refuses beside the fields and keeps what was typed', async () => {
		const { changed } = render();
		const f = await openForm('Web links', '+ Add web link', 'Add web link');
		const [title, url] = inputs(f);
		fill(title, 'Page');
		for (const [typed, message] of [
			['ftp://example.com/file', "a web link's URL must start with http:// or https://, be at most 1000 characters and hold no user name or password"],
			['https://mia:secret@example.com/wiki', "a web link's URL must start with http:// or https://, be at most 1000 characters and hold no user name or password"],
			['https://example.com/wiki', 'this item already has a web link to that URL, titled "Scenario page": remove it first, then add it with the new title']
		]) {
			fill(url, typed);
			net.answer = () => failure(message, 400, 'validation');
			const before = writes().length;
			submit(f);
			// The request first: two of the three messages are the same words, so the alert alone would not show that
			// this submission was sent, nor with what.
			await vi.waitFor(() => expect(writes()).toHaveLength(before + 1));
			expect(writes()[before]).toEqual({ method: 'POST', path: '/api/v1/items/ABC-10/web-links', body: { title: 'Page', url: typed } });
			await vi.waitFor(() => {
				flushSync();
				expect(alertIn(form('Add web link'))).toBe(message);
			});
			// Both fields point at the message, and are marked invalid in more than colour; the URL field keeps its hint.
			const alertId = form('Add web link')!.querySelector('[role="alert"]')!.id;
			const hintId = form('Add web link')!.querySelector('.hint')!.id;
			expect([title.getAttribute('aria-invalid'), title.getAttribute('aria-describedby')]).toEqual(['true', alertId]);
			expect([url.getAttribute('aria-invalid'), url.getAttribute('aria-describedby')]).toEqual(['true', `${hintId} ${alertId}`]);
			expect([title.value, url.value]).toEqual(['Page', typed]);
			await vi.waitFor(() => expect(document.activeElement).toBe(title));
		}
		expect(section('Web links').textContent).toContain('No web links yet.');
		expect(changed).not.toHaveBeenCalled();
		// Cancel drops the draft and the error: a reopened form starts empty.
		button(f, 'Cancel').click();
		flushSync();
		const again = await openForm('Web links', '+ Add web link', 'Add web link');
		expect(inputs(again).map((i) => i.value)).toEqual(['', '']);
		expect(alertIn(again)).toBe('');
	});

	it('shows a failed removal beside the list, keeps the web link and gives focus back to the button that was pressed', async () => {
		render({ webLinks: [webLink({ id: 4, title: 'Scenario page' })] });
		const pressed = () => button(section('Web links'), 'Remove web link Scenario page');
		const g = gate();
		net.answer = async () => {
			await g.opened;
			return failure('database down');
		};
		pressed().focus();
		pressed().click();
		// As in the item link test: focus is dropped to the page at once, as a browser may do for a disabled button.
		pressed().blur();
		expect(document.activeElement).toBe(document.body);
		await vi.waitFor(() => expect(writes()).toEqual([{ method: 'DELETE', path: '/api/v1/items/ABC-10/web-links/4', body: null }]));
		g.release();
		await vi.waitFor(() => {
			flushSync();
			expect(alertIn(section('Web links'))).toBe('Could not remove the web link Scenario page: database down');
		});
		expect(section('Web links').querySelectorAll('li')).toHaveLength(1);
		await vi.waitFor(() => expect(document.activeElement).toBe(pressed()));
	});

	it('after a failed removal whose row a reload took meanwhile, focus goes to the next Remove, else to the add button, not to the page', async () => {
		const first = webLink({ id: 4, title: 'Scenario page' });
		const second = webLink({ id: 9, title: 'Runbook', url: 'https://example.com/runbook' });
		for (const [left, lands] of [
			[[second], 'Remove web link Runbook'],
			[[], '+ Add web link']
		] as const) {
			const { props } = render({ webLinks: [first, second] });
			const g = gate();
			net.answer = async () => {
				await g.opened;
				return failure('database down');
			};
			const pressed = button(section('Web links'), 'Remove web link Scenario page');
			pressed.focus();
			pressed.click();
			await vi.waitFor(() => expect(writes()).toHaveLength(1));
			props.webLinks = [...left];
			flushSync();
			expect(pressed.isConnected).toBe(false);
			expect(document.activeElement).toBe(document.body);
			g.release();
			await vi.waitFor(() => {
				flushSync();
				expect(alertIn(section('Web links'))).toBe('Could not remove the web link Scenario page: database down');
			});
			await vi.waitFor(() => expect(document.activeElement?.getAttribute('aria-label') ?? document.activeElement?.textContent?.trim()).toBe(lands));
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
			net.calls.length = 0;
		}
	});
});
