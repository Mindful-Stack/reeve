// @vitest-environment jsdom
// The template picker of the create forms: the default is preselected and in the field, choosing another replaces
// an untouched field, text the author changed is never replaced without an answer, a kept draft is known by what the
// picker last applied to it (never by matching the list), and the form is told when the list has arrived.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { briefTemplate } from '#lib/test/fixtures.ts';
import Harness from './TemplatePickerHarness.test.svelte';

const net = vi.hoisted(() => ({
	templates: [] as unknown[],
	fail: false,
	levels: [] as (string | null)[],
	/** While set, the list is not answered: the request waits on it. */
	gate: null as Promise<void> | null
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	installFetch(async ({ method, path, url }) => {
		if (method !== 'GET' || path !== '/api/v1/brief-templates') throw new Error(`unexpected ${method} ${path}`);
		net.levels.push(url.searchParams.get('level'));
		if (net.gate) await net.gate;
		return net.fail ? failure('boom') : net.templates;
	});
	return orig();
});

const SCENARIO = '# Scenario\n\n| Step | Who |\n|---|---|\n';
const DEBT = '## Debt\n';
const two = () => [
	briefTemplate({ id: 1, level: 'epic', name: 'Scenario', body: SCENARIO }),
	briefTemplate({ id: 2, level: 'epic', name: 'Tech debt', body: DEBT, is_default: true })
];

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.templates = [];
	net.fail = false;
	net.levels.length = 0;
	net.gate = null;
	vi.useRealTimers();
});

const select = () => document.body.querySelector('select') as HTMLSelectElement | null;
const brief = () => document.body.querySelector('textarea[aria-label="Brief"]') as HTMLTextAreaElement;
const state = () => document.body.querySelector('output[aria-label="Templates"]')!.textContent;
const ask = () => document.body.querySelector('[role="alert"]');
const button = (name: string) => [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === name)!;
const buttons = () => [...(ask()?.querySelectorAll('button') ?? [])].map((b) => b.textContent?.trim());
function choose(id: string) {
	select()!.value = id;
	select()!.dispatchEvent(new Event('change', { bubbles: true }));
	flushSync();
}
function type(v: string) {
	brief().value = v;
	brief().dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
}
function start(start = '', prefill = true, kept?: { id: string; body: string }) {
	app = mount(Harness, { target: document.body, props: { level: 'epic', start, prefill, kept } });
	flushSync();
}
/** Mounts the picker and waits until it says the list has arrived (or failed to). `kept` is what the picker last
 *  applied to a kept draft, as the form remembers it. */
async function open(text = '', prefill = true, kept?: { id: string; body: string }) {
	start(text, prefill, kept);
	await vi.waitFor(() => {
		flushSync();
		expect(state()).toBe('ready');
	});
}

describe('TemplatePicker', () => {
	it("asks for its level's templates, preselects the default and puts its body in the field", async () => {
		net.templates = two();
		await open();
		expect(net.levels).toEqual(['epic']);
		expect([...select()!.options].map((o) => o.textContent?.trim())).toEqual(['No template', 'Scenario', 'Tech debt (default)']);
		expect(select()!.value).toBe('2');
		expect(brief().value).toBe(DEBT);
	});

	it('replaces an untouched field when another entry is chosen, and empties it for No template', async () => {
		net.templates = two();
		await open();
		choose('1');
		expect(brief().value).toBe(SCENARIO);
		expect(ask()).toBeNull();
		choose('');
		expect(brief().value).toBe('');
		choose('2');
		expect(brief().value).toBe(DEBT);
	});

	it('never replaces text the author changed without an answer', async () => {
		net.templates = two();
		await open();
		type(`${DEBT}What I wrote.`);
		choose('1');
		// Nothing changed yet: the text, and the select is back on the entry the text came from.
		expect(brief().value).toBe(`${DEBT}What I wrote.`);
		expect(select()!.value).toBe('2');
		expect(ask()!.textContent).toContain('Replace what you have written with the template "Scenario"?');
		expect(buttons()).toEqual(['Replace', 'Keep my text']);

		button('Keep my text').click();
		flushSync();
		expect(ask()).toBeNull();
		expect(brief().value).toBe(`${DEBT}What I wrote.`);
		expect(select()!.value).toBe('2');

		choose('1');
		button('Replace').click();
		flushSync();
		expect(ask()).toBeNull();
		expect(brief().value).toBe(SCENARIO);
		expect(select()!.value).toBe('1');
	});

	it('asks before No template empties changed text, with buttons that say what each does', async () => {
		net.templates = two();
		await open();
		type('Mine');
		choose('');
		expect(brief().value).toBe('Mine');
		expect(ask()!.textContent).toContain('Remove what you have written?');
		expect(buttons()).toEqual(['Remove my text', 'Keep my text']);
		button('Keep my text').click();
		flushSync();
		expect(brief().value).toBe('Mine');
		expect(select()!.value).toBe('2');
		choose('');
		button('Remove my text').click();
		flushSync();
		expect(ask()).toBeNull();
		expect(brief().value).toBe('');
		expect(select()!.value).toBe('');
	});

	it('puts focus back on the select when the question is answered, unless the author moved it', async () => {
		net.templates = two();
		await open();
		const answer = (name: string) => {
			button(name).focus();
			button(name).click();
			flushSync();
		};
		type('Mine');
		choose('1');
		answer('Keep my text');
		await vi.waitFor(() => expect(document.activeElement).toBe(select()));
		choose('1');
		answer('Replace');
		await vi.waitFor(() => expect(document.activeElement).toBe(select()));
		type('Mine again');
		choose('');
		answer('Remove my text');
		await vi.waitFor(() => expect(document.activeElement).toBe(select()));
		// Focus the author moved to the brief while the question was open stays there.
		type('Mine once more');
		choose('1');
		button('Keep my text').click();
		brief().focus();
		flushSync();
		await new Promise((r) => setTimeout(r));
		expect(ask()).toBeNull();
		expect(document.activeElement).toBe(brief());
	});

	it("says, while a template is chosen, that its text is the author's to change, and ties the line to the select", async () => {
		net.templates = two();
		await open();
		const OWN = "The template's text is in the brief below. Change it as you like; choosing another template never replaces what you have written without asking.";
		const described = () => {
			const id = select()!.getAttribute('aria-describedby');
			return id ? document.getElementById(id)!.textContent!.replace(/\s+/g, ' ').trim() : null;
		};
		expect(described()).toBe(OWN);
		// "No template": there is no template's text to speak of.
		choose('');
		expect(described()).toBeNull();
		expect(document.body.textContent).not.toContain('Change it as you like');
		choose('1');
		expect(described()).toBe(OWN);
	});

	it('leaves text of the author that was there before the templates arrived, and shows No template', async () => {
		net.templates = two();
		await open('Typed before');
		expect(brief().value).toBe('Typed before');
		expect(select()!.value).toBe('');
	});

	it('knows a kept draft that is still what the picker applied: shows that entry and replaces without asking', async () => {
		net.templates = two();
		await open(DEBT, false, { id: '2', body: DEBT });
		expect(brief().value).toBe(DEBT);
		expect(select()!.value).toBe('2');
		choose('1');
		expect(ask()).toBeNull();
		expect(brief().value).toBe(SCENARIO);
	});

	// Catches ownership guessed from the list: a kept draft the author changed until it equals another template's
	// body is the author's text, shown as No template, and No template or another entry asks before replacing it.
	it("takes a kept draft the author changed for the author's, even when it equals a template's body", async () => {
		net.templates = two();
		await open(SCENARIO, false, { id: '2', body: DEBT });
		expect(select()!.value).toBe('');
		choose('2');
		expect(ask()!.textContent).toContain('Replace what you have written with the template "Tech debt"?');
		expect(brief().value).toBe(SCENARIO);
	});

	// Catches the same guess for a fresh picker: text there before the list arrived is the author's, whatever it equals.
	it("takes text that equals a template's body, there before the list arrived, for the author's", async () => {
		net.templates = two();
		await open(SCENARIO);
		expect(brief().value).toBe(SCENARIO);
		expect(select()!.value).toBe('');
		choose('2');
		expect(ask()).not.toBeNull();
		expect(brief().value).toBe(SCENARIO);
	});

	it('is not ready until the list has arrived, and then is', async () => {
		let release!: () => void;
		net.gate = new Promise<void>((r) => (release = r));
		net.templates = two();
		start();
		await vi.waitFor(() => expect(net.levels).toHaveLength(1));
		flushSync();
		expect(state()).toBe('loading');
		expect(select()).toBeNull();
		expect(brief().value).toBe('');
		release();
		await vi.waitFor(() => {
			flushSync();
			expect(state()).toBe('ready');
		});
		expect(brief().value).toBe(DEBT);
	});

	it('does not prefill when told not to, and still offers the templates', async () => {
		net.templates = two();
		await open('', false);
		expect(brief().value).toBe('');
		expect(select()!.value).toBe('');
		choose('2');
		expect(brief().value).toBe(DEBT);
	});

	it('shows nothing when the level has no template, and is ready', async () => {
		await open();
		expect(select()).toBeNull();
		expect(brief().value).toBe('');
	});

	it('says so when the templates cannot be loaded, leaves the field alone, and is ready', async () => {
		net.fail = true;
		await open('Mine');
		expect(select()).toBeNull();
		expect(document.body.textContent).toContain('Templates could not be loaded');
		expect(brief().value).toBe('Mine');
	});

	it('stops waiting after 10 seconds for a list that never arrives, and is ready as if the load failed', async () => {
		const { settle } = await import('#lib/test/net.ts');
		// Only the timers: the fake server and `settle` run on promises, which stay real.
		vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
		net.gate = new Promise<void>(() => {}); // never settles
		net.templates = two();
		start();
		await settle();
		flushSync();
		expect(net.levels).toEqual(['epic']);
		vi.advanceTimersByTime(9_999);
		flushSync();
		expect(state()).toBe('loading');
		vi.advanceTimersByTime(1);
		flushSync();
		expect(state()).toBe('ready');
		expect(document.body.textContent).toContain('Templates could not be loaded');
		expect(select()).toBeNull();
		expect(brief().value).toBe('');
	});
});
