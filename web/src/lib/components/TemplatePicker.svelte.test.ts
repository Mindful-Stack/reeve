// @vitest-environment jsdom
// The template picker of the create forms: the default is preselected and in the field, choosing another replaces
// an untouched field, text the author changed is never replaced without an answer, a draft that is still a
// template's own text is known as that template, and the form is told when the list has arrived.
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
function start(start = '', prefill = true) {
	app = mount(Harness, { target: document.body, props: { level: 'epic', start, prefill } });
	flushSync();
}
/** Mounts the picker and waits until it says the list has arrived (or failed to). */
async function open(text = '', prefill = true) {
	start(text, prefill);
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

	it('leaves text of the author that was there before the templates arrived, and shows No template', async () => {
		net.templates = two();
		await open('Typed before');
		expect(brief().value).toBe('Typed before');
		expect(select()!.value).toBe('');
	});

	it("knows a kept draft that is still the default's text: shows the default and replaces without asking", async () => {
		net.templates = two();
		await open(DEBT);
		expect(brief().value).toBe(DEBT);
		expect(select()!.value).toBe('2');
		choose('1');
		expect(ask()).toBeNull();
		expect(brief().value).toBe(SCENARIO);
	});

	it("knows a kept draft that is another template's text, and prefers the default when two share a body", async () => {
		net.templates = two();
		await open(SCENARIO);
		expect(select()!.value).toBe('1');
		choose('');
		expect(ask()).toBeNull();
		expect(brief().value).toBe('');
		unmount(app!);
		document.body.innerHTML = '';
		// A twin of the default, listed before it: the default is still the one shown.
		net.templates = [briefTemplate({ id: 7, level: 'epic', name: 'Also debt', body: DEBT }), ...two()];
		await open(DEBT);
		expect(select()!.value).toBe('2');
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
