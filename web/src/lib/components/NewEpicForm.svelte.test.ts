// @vitest-environment jsdom
// The "New epic" form: a double submit sends one POST, a failure keeps the draft and the key, a success resets the form
// and replaces the key, Cancel-free; the parent is fixed on an initiative page and picked on the epics index.
import { flushSync, mount, unmount, type ComponentProps } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { briefTemplate, config, initiative } from '#lib/test/fixtures.ts';
import { choose, offered, picker } from '#lib/test/picker.ts';
import { session } from '#lib/me.svelte.ts';
import NewEpicForm from './NewEpicForm.svelte';

type Posted = { idempotency_key: string; title: string; brief: string | null; initiative: string | null };

const net = vi.hoisted(() => ({
	posts: [] as Posted[],
	templates: [] as unknown[],
	templatesFail: false,
	/** While set, the templates are not answered: the request waits on it. */
	gate: null as Promise<void> | null,
	state: { failNext: 0, rejectNext: 0 }
}));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('#lib/test/net.ts');
	const { epic } = await import('#lib/test/fixtures.ts');
	installFetch(async ({ method, path, body }) => {
		if (method === 'GET' && path === '/api/v1/brief-templates') {
			if (net.gate) await net.gate;
			return net.templatesFail ? failure('no templates today') : net.templates;
		}
		if (method === 'POST' && path === '/api/v1/attachments')
			return { id: 'img-e', url: '/api/v1/attachments/img-e', filename: 'shot.png', content_type: 'image/png', byte_size: 4, uploaded_by: 'dan@example.com', uploaded_at: '2026-11-10T09:00:00Z' };
		if (method !== 'POST' || path !== '/api/v1/epics') throw new Error(`unexpected ${method} ${path}`);
		const posted = body as Posted;
		net.posts.push(posted);
		await new Promise((r) => setTimeout(r, 5));
		if (net.state.rejectNext > 0) {
			net.state.rejectNext--;
			throw new TypeError('Failed to fetch');
		}
		if (net.state.failNext > 0) {
			net.state.failNext--;
			return failure('boom');
		}
		return epic({ key: 'SMK-9', title: posted.title });
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
	net.state.rejectNext = 0;
	net.templates = [];
	net.templatesFail = false;
	net.gate = null;
	session.config = null;
});

const form = () => document.body.querySelector('form[aria-label="New epic"]') as HTMLFormElement;
function field(label: string): HTMLInputElement {
	const l = [...form().querySelectorAll('label')].find((x) => x.textContent?.trim().startsWith(label));
	return l!.querySelector('input,select,textarea') as HTMLInputElement;
}
function type(el: HTMLInputElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	el.dispatchEvent(new Event('change', { bubbles: true }));
	flushSync();
}
function mountOnly(props: Omit<ComponentProps<typeof NewEpicForm>, 'oncreated'> = {}) {
	const oncreated = vi.fn(async () => {});
	app = mount(NewEpicForm, { target: document.body, props: { oncreated, ...props } });
	flushSync();
	return oncreated;
}
const submitButton = () => form().querySelector('button[type="submit"]') as HTMLButtonElement;
/** The form's templates have loaded (or failed to): its submit button no longer says so. */
async function loaded() {
	await vi.waitFor(() => {
		flushSync();
		expect(submitButton().textContent).not.toContain('Loading templates');
	});
}
async function mountForm(props: Omit<ComponentProps<typeof NewEpicForm>, 'oncreated'> = {}) {
	const oncreated = mountOnly(props);
	await loaded();
	return oncreated;
}
const alertText = () => form().querySelector('[role="alert"]')?.textContent ?? '';

describe('NewEpicForm', () => {
	it('gives the brief the room a template needs, as the brief editor of an epic does', async () => {
		await mountForm();
		expect((field('Brief') as unknown as HTMLTextAreaElement).rows).toBe(12);
	});

	it('returns focus to Title after a create and after a failed one', async () => {
		await mountForm();
		type(field('Title'), 'Billing');
		(form().querySelector('button[type="submit"]') as HTMLButtonElement).focus();
		form().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(net.posts).toHaveLength(1);
			expect(document.activeElement).toBe(field('Title'));
		});
		net.state.failNext = 1;
		await loaded();
		type(field('Title'), 'Again');
		(form().querySelector('button[type="submit"]') as HTMLButtonElement).focus();
		form().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(alertText()).toContain('boom');
			expect(document.activeElement).toBe(field('Title'));
		});
	});

	it('leaves focus where the user moved it during a slow create', async () => {
		await mountForm();
		type(field('Title'), 'Billing');
		form().requestSubmit();
		const other = document.createElement('input');
		document.body.append(other);
		other.focus();
		await vi.waitFor(() => expect(net.posts).toHaveLength(1));
		await new Promise((r) => setTimeout(r, 40));
		flushSync();
		expect(document.activeElement).toBe(other);
	});

	it("lists the initiatives in the planning page's order, not the server's", async () => {
		session.config = config({ horizon_buckets: ['Now', 'Next'] });
		const ini = (key: string, title: string, horizon: string, target_month: string | null) =>
			initiative({ key, title, horizon, target_month });
		// Server order is creation order; the planning page shows Now before Next, then target month, then title.
		const initiatives = [
			ini('ABC-1', 'Later thing', 'Next', null),
			ini('ABC-2', 'Unplanned', 'Someday', null),
			ini('ABC-3', 'Beta', 'Now', null),
			ini('ABC-4', 'Alpha', 'Now', '2026-12')
		];
		await mountForm({ initiatives });
		expect(offered(form(), 'Initiative')).toEqual(['No initiative', 'ABC-4 Alpha', 'ABC-3 Beta', 'ABC-1 Later thing', 'ABC-2 Unplanned']);
	});

	it('sends the fixed initiative and no parent picker on an initiative page', async () => {
		const oncreated = await mountForm({ initiative: 'ABC-3' });
		expect([...form().querySelectorAll('label')].some((l) => l.textContent?.includes('Initiative'))).toBe(false);
		type(field('Title'), 'An epic');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[0]).toEqual({ idempotency_key: expect.any(String), title: 'An epic', brief: null, initiative: 'ABC-3' });
	});

	it('sends one request for a double submit', async () => {
		const oncreated = await mountForm({ initiative: 'ABC-3' });
		type(field('Title'), 'An epic');
		form().requestSubmit();
		form().requestSubmit();
		await new Promise((r) => setTimeout(r, 0)); // the refused second send settles before the 5 ms answer
		flushSync();
		expect(field('Title').disabled).toBe(true); // still locked while the first POST is in flight
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts).toHaveLength(1);
	});

	it('keeps the draft and the key after a failure, then reuses the key for the retry', async () => {
		const oncreated = await mountForm({ initiative: 'ABC-3' });
		type(field('Title'), 'An epic');
		net.state.failNext = 1;
		form().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(alertText()).toContain('boom');
		});
		expect(field('Title').value).toBe('An epic');
		expect(oncreated).not.toHaveBeenCalled();
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[1].idempotency_key).toBe(net.posts[0].idempotency_key);
	});

	it('keeps the key when the request is rejected before an answer', async () => {
		const oncreated = await mountForm({ initiative: 'ABC-3' });
		type(field('Title'), 'An epic');
		net.state.rejectNext = 1;
		form().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(alertText()).toContain('Failed to fetch');
		});
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[1].idempotency_key).toBe(net.posts[0].idempotency_key);
	});

	it('resets the form and replaces the key after a success', async () => {
		const oncreated = await mountForm({ initiative: 'ABC-3' });
		type(field('Title'), 'An epic');
		type(field('Brief'), 'Why');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		flushSync();
		expect(field('Title').value).toBe('');
		expect(field('Brief').value).toBe('');
		await loaded();
		type(field('Title'), 'An epic');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(2));
		expect(net.posts[1].idempotency_key).not.toBe(net.posts[0].idempotency_key);
	});

	it('mints a new key when the text changes after a failed send', async () => {
		const oncreated = await mountForm({ initiative: 'ABC-3' });
		type(field('Title'), 'An epic');
		net.state.failNext = 1;
		form().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(alertText()).toContain('boom');
		});
		type(field('Title'), 'Another epic');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[1].idempotency_key).not.toBe(net.posts[0].idempotency_key);
	});

	it('picks the parent from a picker on the epics index, keeping it after a success', async () => {
		const oncreated = await mountForm({ initiatives: [initiative({ key: 'ABC-1', title: 'One' }), initiative({ key: 'ABC-2', title: 'Two' })] });
		choose(form(), 'Initiative', 'ABC-2');
		type(field('Title'), 'An epic');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[0].initiative).toBe('ABC-2');
		flushSync();
		expect(picker(form(), 'Initiative').value).toBe('ABC-2 Two');
	});

	const debt = () => [briefTemplate({ id: 5, level: 'epic', name: 'Tech debt', body: '## Debt\n', is_default: true })];

	it('starts the brief from the default epic template, sends what the author made of it, and starts over after a create', async () => {
		net.templates = debt();
		await mountForm({ initiative: 'ABC-1' });
		expect(field('Brief').value).toBe('## Debt\n');
		expect(field('Template').value).toBe('5');
		type(field('Title'), 'Pay it down');
		type(field('Brief'), '## Debt\nThe old importer.');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(1));
		// The brief, and nothing about templates: the server is not asked to apply one.
		expect(net.posts[0]).toEqual({ idempotency_key: expect.any(String), title: 'Pay it down', brief: '## Debt\nThe old importer.', initiative: 'ABC-1' });
		// The form is ready for another epic: a fresh draft, with the default in the field again.
		await vi.waitFor(() => {
			flushSync();
			expect(field('Title').value).toBe('');
			expect(field('Brief').value).toBe('## Debt\n');
		});
	});

	it('sends an emptied brief as none, and it stays none: nothing asks the server for the default', async () => {
		net.templates = debt();
		await mountForm({ initiative: 'ABC-1' });
		// The default is in the field first, so what follows removes prefilled text, not nothing.
		expect(field('Brief').value).toBe('## Debt\n');
		type(field('Title'), 'Bare');
		type(field('Brief'), '');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(1));
		expect(net.posts[0]).toEqual({ idempotency_key: expect.any(String), title: 'Bare', brief: null, initiative: 'ABC-1' });
	});

	it('cannot be sent before the templates have loaded, and a lost answer is retried as the same create', async () => {
		const { settle } = await import('#lib/test/net.ts');
		let release!: () => void;
		net.gate = new Promise<void>((r) => (release = r));
		net.templates = debt();
		mountOnly({ initiative: 'ABC-1' });
		type(field('Title'), 'Early');
		// The list has not arrived: the button says so, and a submit by the keyboard is refused as well.
		expect(submitButton().disabled).toBe(true);
		expect(submitButton().textContent).toContain('Loading templates');
		form().requestSubmit();
		await settle();
		flushSync();
		expect(net.posts).toHaveLength(0);

		release();
		await loaded();
		expect(field('Brief').value).toBe('## Debt\n');
		// The server stores the epic and its answer is lost on the way back.
		net.state.rejectNext = 1;
		form().requestSubmit();
		await vi.waitFor(() => {
			flushSync();
			expect(alertText()).toContain('Failed to fetch');
		});
		// Nothing arrives late to change the draft: the retry has the same content and so the same key.
		expect(field('Brief').value).toBe('## Debt\n');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(2));
		expect(net.posts[1]).toEqual(net.posts[0]);
	});

	it('works as before when the templates cannot be loaded', async () => {
		net.templatesFail = true;
		await mountForm({ initiative: 'ABC-1' });
		expect(form().textContent).toContain('Templates could not be loaded');
		type(field('Title'), 'Plain');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(1));
		expect(net.posts[0]).toEqual({ idempotency_key: expect.any(String), title: 'Plain', brief: null, initiative: 'ABC-1' });
	});
});

describe('NewEpicForm images', () => {
	it('sends the image shown in its brief, and empties the row after the create', async () => {
		const { file } = await import('#lib/test/events.ts');
		app = mount(NewEpicForm, { target: document.body, props: { initiative: 'ABC-1', oncreated: () => {} } });
		flushSync();
		await loaded();
		const title = document.body.querySelector<HTMLInputElement>('input:not([type=file])')!;
		title.value = 'An epic';
		title.dispatchEvent(new Event('input', { bubbles: true }));
		const input = document.body.querySelector<HTMLInputElement>('input[type=file]')!;
		Object.defineProperty(input, 'files', { value: [file('shot.png')], configurable: true });
		input.dispatchEvent(new Event('change', { bubbles: true }));
		await vi.waitFor(() => expect(document.body.querySelector('textarea')!.value).toContain('/api/v1/attachments/img-e'));
		document.body.querySelector('form')!.dispatchEvent(new Event('submit', { cancelable: true }));
		await vi.waitFor(() => expect(net.posts).toHaveLength(1));
		expect(net.posts[0]).toMatchObject({ attachments: ['img-e'] });
		await vi.waitFor(() => expect(document.body.querySelector('.uploads li')).toBeNull());
	});
});
