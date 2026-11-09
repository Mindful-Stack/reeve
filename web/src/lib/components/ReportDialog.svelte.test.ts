// @vitest-environment jsdom
// The Report dialog: the type is required, a template survives edits, the epic chip can be dropped, a double submit and a
// lost answer never file twice, the duplicate check drops stale answers and survives failing, and leaving to look at a
// hit or pressing Escape keeps the draft.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SimilarStory, Story } from '#lib/api/types.ts';
import { fullLabel } from '#lib/board.ts';
import { discardDraft, openReport, report } from '#lib/report.svelte.ts';
import ReportDialog from './ReportDialog.svelte';

type Posted = { idempotency_key: string; title: string; kind: Story['kind']; epic: string | null; description: string };
type Deferred = { q: string; resolve: (hits: SimilarStory[]) => void; fail: () => void };

const net = vi.hoisted(() => ({
	posts: [] as Posted[],
	similarCalls: [] as string[],
	// What the next POSTs do: 'ok', 'throw' (the answer is lost), 'refuse' or 'error' (the server answered, wrote nothing).
	post: [] as string[],
	// When set, a similar call waits for the test to answer it.
	hold: false,
	held: [] as Deferred[],
	hits: [] as SimilarStory[],
	similarFails: false
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure, json } = await import('#lib/test/net.ts');
	const { story } = await import('#lib/test/fixtures.ts');
	installFetch(async ({ method, path, url, body }) => {
		if (method === 'GET' && path === '/api/v1/stories/similar') {
			const q = url.searchParams.get('q') ?? '';
			net.similarCalls.push(q);
			if (net.similarFails) return failure('boom');
			if (net.hold) return new Promise((resolve) => net.held.push({ q, resolve: (h) => resolve(h), fail: () => resolve(failure('boom')) }));
			return net.hits;
		}
		if (method === 'POST' && path === '/api/v1/stories') {
			const posted = body as Posted;
			net.posts.push(posted);
			await new Promise((r) => setTimeout(r, 5));
			const mode = net.post.shift() ?? 'ok';
			if (mode === 'throw') throw new TypeError('network down');
			if (mode === 'refuse') return json({ outcome: 'refused', reasons: [{ reason: 'no_criteria' }], overridable: false });
			if (mode === 'error') return failure('boom');
			return story({ key: 'SMK-9', title: posted.title, kind: posted.kind, epic: posted.epic });
		}
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

const hit = (key: string, title: string, over: Partial<SimilarStory> = {}): SimilarStory => ({
	key,
	title,
	kind: 'bug',
	epic: null,
	delivery: 'todo',
	resolution: null,
	...over
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	vi.useRealTimers();
	discardDraft();
	document.body.innerHTML = '';
	document.body.removeAttribute('tabindex');
	Object.assign(net, { hold: false, similarFails: false, hits: [] });
	net.posts.length = 0;
	net.similarCalls.length = 0;
	net.post.length = 0;
	net.held.length = 0;
});

const ctx = { path: '/board', label: 'the board', epic: null };
function mountDialog(c: Parameters<typeof openReport>[0] = ctx) {
	openReport(c);
	const oncreated = vi.fn();
	const onviewstory = vi.fn();
	app = mount(ReportDialog, { target: document.body, props: { oncreated, onviewstory } });
	flushSync();
	return { oncreated, onviewstory };
}
const form = () => document.body.querySelector('form[aria-label="Report"]') as HTMLFormElement;
const title = () => form().querySelector('input:not([type=radio])') as HTMLInputElement;
const desc = () => form().querySelector('textarea') as HTMLTextAreaElement;
const alertText = () => form().querySelector('[role=alert]')?.textContent?.trim() ?? null;
const radio = (label: string) =>
	[...form().querySelectorAll('label.type')].find((l) => l.textContent?.includes(label))!.querySelector('input') as HTMLInputElement;
const button = (label: string) => [...form().querySelectorAll('button')].find((b) => b.textContent?.trim() === label) as HTMLButtonElement | undefined;
function type(el: HTMLInputElement | HTMLTextAreaElement, v: string) {
	el.value = v;
	el.dispatchEvent(new Event('input', { bubbles: true }));
	flushSync();
}
function choose(label: string) {
	radio(label).click();
	flushSync();
}
const settle = () => new Promise((r) => setTimeout(r, 20)).then(() => flushSync());

describe('ReportDialog', () => {
	it('refuses a submit with no kind', async () => {
		mountDialog();
		type(title(), 'The board is slow');
		form().requestSubmit();
		flushSync();
		expect(alertText()).toBe('Choose what kind of thing this is.');
		await settle();
		expect(net.posts).toHaveLength(0);
	});

	it('fills the template for a type, keeps an edit across a switch, replaces an untouched one', () => {
		mountDialog();
		choose("Something's broken");
		expect(desc().value).toContain('What happened:');
		choose('Idea or change');
		expect(desc().value).toBe('What and why:\n');
		type(desc(), 'My own words');
		choose("Something's broken");
		expect(desc().value).toBe('My own words');
	});

	it('sends to the chip epic, and to triage after "Send to triage instead"; no chip off an epic page', async () => {
		const { oncreated } = mountDialog({ path: '/epics/ABC-30', label: 'Payments', epic: { key: 'ABC-30', title: 'Payments' } });
		expect(form().textContent).toContain('Files directly under ABC-30 Payments, skipping triage');
		button('Send to triage instead')!.click();
		flushSync();
		expect(form().textContent).toContain('Goes to triage; someone will place it.');
		type(title(), 'Refund totals are off');
		choose("Something's broken");
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[0].epic).toBeNull();
	});

	it('sends the chip epic when kept, and renders no chip on a page that is not an epic', async () => {
		const { oncreated } = mountDialog({ path: '/epics/ABC-30', label: 'Payments', epic: { key: 'ABC-30', title: 'Payments' } });
		type(title(), 'Refund totals are off');
		choose('Idea or change');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[0].epic).toBe('ABC-30');
		unmount(app!);
		document.body.innerHTML = '';
		mountDialog();
		expect(form().querySelector('.chip')).toBeNull();
		type(title(), 'Another report here');
		choose('Idea or change');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(2));
		expect(net.posts[1].epic).toBeNull();
	});

	it('sends one request for a double submit', async () => {
		const { oncreated } = mountDialog();
		type(title(), 'Double click report');
		choose('Idea or change');
		form().requestSubmit();
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts).toHaveLength(1);
	});

	it('keeps the draft after a network failure and retries with the identical body and key', async () => {
		const { oncreated } = mountDialog();
		type(title(), 'Lost answer report');
		choose('Idea or change');
		net.post.push('throw');
		form().requestSubmit();
		await vi.waitFor(() => expect(alertText()).not.toBeNull());
		expect(report.draft).not.toBeNull();
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts).toHaveLength(2);
		expect(net.posts[1]).toEqual(net.posts[0]);
	});

	it('checks similar work after 300 ms, drops a stale answer, and survives a failing check', async () => {
		vi.useFakeTimers();
		mountDialog();
		net.hold = true;
		type(title(), 'Board is slow');
		await vi.advanceTimersByTimeAsync(299);
		expect(net.similarCalls).toHaveLength(0);
		await vi.advanceTimersByTimeAsync(1);
		expect(net.similarCalls).toEqual(['Board is slow']);
		type(title(), 'Board is slow today');
		await vi.advanceTimersByTimeAsync(300);
		expect(net.similarCalls).toHaveLength(2);
		net.held[1].resolve([hit('ABC-2', 'newer answer')]);
		await vi.advanceTimersByTimeAsync(10);
		net.held[0].resolve([hit('ABC-1', 'older answer')]);
		await vi.advanceTimersByTimeAsync(10);
		flushSync();
		expect(form().textContent).toContain('newer answer');
		expect(form().textContent).not.toContain('older answer');
		vi.useRealTimers();

		net.hold = false;
		net.similarFails = true;
		type(title(), 'Board is slow tomorrow');
		await vi.waitFor(() => expect(form().textContent).toContain("Couldn't check similar work. You can still submit."));
		choose('Idea or change');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(1));
	});

	it('closes without clearing the draft when a hit is viewed, and remembers it', async () => {
		net.hits = [hit('ABC-412', 'the board is slow')];
		const { onviewstory } = mountDialog();
		type(title(), 'Board is slow');
		await vi.waitFor(() => expect(form().querySelector('[data-hit="ABC-412"]')).not.toBeNull());
		(form().querySelector('[data-hit="ABC-412"]') as HTMLButtonElement).click();
		flushSync();
		expect(onviewstory).toHaveBeenCalledWith('ABC-412');
		expect(report.open).toBe(false);
		expect(report.draft?.title).toBe('Board is slow');
		expect(report.draft?.leftFrom).toBe('ABC-412');
		expect(document.body.querySelector('form')).toBeNull();
	});

	it('keeps the draft on Escape, and clears it on "Discard draft"', () => {
		mountDialog();
		type(title(), 'Typed then escaped');
		document.body.querySelector('dialog')!.dispatchEvent(new Event('cancel', { cancelable: true }));
		flushSync();
		expect(report.open).toBe(false);
		expect(report.draft?.title).toBe('Typed then escaped');
		report.open = true;
		flushSync();
		button('Discard draft')!.click();
		flushSync();
		expect(report.draft).toBeNull();
		expect(document.body.querySelector('form')).toBeNull();
	});

	it('calls oncreated with the story and the chip epic title, and renders nothing once the draft is gone', async () => {
		const { oncreated } = mountDialog({ path: '/epics/ABC-30', label: 'Payments', epic: { key: 'ABC-30', title: 'Payments' } });
		type(title(), 'Refund totals are off');
		choose('Idea or change');
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(oncreated).toHaveBeenCalledWith(expect.objectContaining({ key: 'SMK-9' }), 'Payments');
		expect(report.draft).toBeNull();
		expect(document.body.querySelector('form')).toBeNull();
	});

	it('marks a lost answer uncertain, retries it unchanged, and asks before sending an edit', async () => {
		const { oncreated } = mountDialog();
		type(title(), 'Maybe filed report');
		choose('Idea or change');
		net.post.push('throw');
		form().requestSubmit();
		await vi.waitFor(() => expect(report.draft?.uncertain).toBe(true));
		expect(alertText()).toBe("We couldn't confirm the report was filed. Send again to retry safely.");

		net.post.push('throw');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(2));
		expect(net.posts[1]).toEqual(net.posts[0]);
		await settle();

		net.hits = [hit('ABC-5', 'maybe filed report')];
		type(title(), 'Maybe filed report, edited');
		form().requestSubmit();
		flushSync();
		expect(alertText()).toBe('Your last report may have been filed. Check before sending again?');
		expect(button('Send anyway')).toBeDefined();
		await vi.waitFor(() => expect(form().querySelector('[data-hit="ABC-5"]')).not.toBeNull());
		expect(net.posts).toHaveLength(2);
		button('Send anyway')!.click();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts).toHaveLength(3);
		expect(net.posts[2].idempotency_key).not.toBe(net.posts[0].idempotency_key);
	});

	it('does not mark a refused or error-envelope answer uncertain', async () => {
		mountDialog();
		type(title(), 'Refused report here');
		choose('Idea or change');
		net.post.push('refuse', 'error');
		form().requestSubmit();
		await vi.waitFor(() => expect(alertText()).not.toBeNull());
		expect(report.draft?.uncertain).toBe(false);
		await settle();
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(2));
		await settle();
		expect(report.draft?.uncertain).toBe(false);
		expect(alertText()).toBe('boom');
	});

	it('re-runs the check on resume and focuses the hit the reporter left from', async () => {
		net.hits = [hit('ABC-411', 'other'), hit('ABC-412', 'board is slow')];
		openReport(ctx);
		report.draft!.title = 'Board is slow';
		report.draft!.leftFrom = 'ABC-412';
		app = mount(ReportDialog, { target: document.body, props: { oncreated: vi.fn(), onviewstory: vi.fn() } });
		flushSync();
		await vi.waitFor(() => expect(document.activeElement).toBe(form().querySelector('[data-hit="ABC-412"]')));
		expect(net.similarCalls).toEqual(['Board is slow']);
		expect(report.draft?.leftFrom).toBeNull();
	});

	it('shows the kind glyph on a hit and the epic glyph on the chip, and counts code points for the minimum', async () => {
		net.hits = [hit('ABC-7', 'a bug', { kind: 'bug' })];
		mountDialog({ path: '/epics/ABC-30', label: 'Payments', epic: { key: 'ABC-30', title: 'Payments' } });
		expect(form().querySelector('.chip .glyph')?.textContent).toBe('■');
		// Seven emoji are 14 UTF-16 units but 7 code points: below the minimum, no check.
		type(title(), '😀😀😀😀😀😀😀');
		await settle();
		await new Promise((r) => setTimeout(r, 350));
		expect(net.similarCalls).toHaveLength(0);
		type(title(), '😀😀😀😀😀😀😀😀');
		await vi.waitFor(() => expect(net.similarCalls).toHaveLength(1));
		await vi.waitFor(() => expect(form().querySelector('[data-hit="ABC-7"] .glyph')?.textContent).toBe('▲'));
	});

	it('shows the refusal reasons, not "refused", when the server refuses a create', async () => {
		mountDialog();
		type(title(), 'Refused report here');
		choose('Idea or change');
		net.post.push('refuse');
		form().requestSubmit();
		await vi.waitFor(() => expect(alertText()).toBe(fullLabel({ reason: 'no_criteria' })));
	});

	it('shows no error from a previous draft on a new one, but keeps the uncertain notice on a kept draft', async () => {
		mountDialog();
		type(title(), 'Lost answer report');
		choose('Idea or change');
		net.post.push('throw');
		form().requestSubmit();
		await vi.waitFor(() => expect(alertText()).not.toBeNull());
		report.open = false;
		flushSync();
		report.open = true;
		flushSync();
		expect(alertText()).toBe("We couldn't confirm the report was filed. Send again to retry safely.");
		button('Discard draft')!.click();
		flushSync();
		openReport(ctx);
		flushSync();
		expect(form()).not.toBeNull();
		expect(alertText()).toBeNull();
	});

	it('puts focus back on the title after a failed send', async () => {
		mountDialog();
		type(title(), 'Failing report here');
		choose('Idea or change');
		net.post.push('error');
		title().focus();
		form().requestSubmit();
		flushSync();
		// A browser drops focus from a field that becomes disabled; jsdom keeps it, so move it to the page by hand.
		document.body.tabIndex = -1;
		document.body.focus();
		expect(document.activeElement).toBe(document.body);
		await vi.waitFor(() => expect(alertText()).toBe('boom'));
		await vi.waitFor(() => expect(document.activeElement).toBe(title()));
	});

	it('sends one check for two keystrokes 100 ms apart, with the last query', async () => {
		vi.useFakeTimers();
		mountDialog();
		type(title(), 'Board is slo');
		await vi.advanceTimersByTimeAsync(100);
		type(title(), 'Board is slow');
		await vi.advanceTimersByTimeAsync(300);
		expect(net.similarCalls).toEqual(['Board is slow']);
	});

	it('never renders an answer held past close, and a reopened dialog waits for its own answer to refocus', async () => {
		vi.useFakeTimers();
		mountDialog();
		net.hold = true;
		type(title(), 'Board is slow');
		await vi.advanceTimersByTimeAsync(300);
		report.draft!.leftFrom = 'ABC-1';
		report.open = false;
		flushSync();
		net.held[0].resolve([hit('ABC-1', 'old answer')]);
		await vi.advanceTimersByTimeAsync(10);
		report.open = true;
		flushSync();
		await vi.advanceTimersByTimeAsync(10);
		expect(net.similarCalls).toHaveLength(2);
		expect(form().querySelector('[data-hit]')).toBeNull();
		expect(form().textContent).not.toContain('old answer');
		expect(report.draft?.leftFrom).toBe('ABC-1');
		net.held[1].resolve([hit('ABC-1', 'new answer')]);
		await vi.advanceTimersByTimeAsync(10);
		flushSync();
		expect(document.activeElement).toBe(form().querySelector('[data-hit="ABC-1"]'));
		expect(form().textContent).toContain('new answer');
	});

	it('shows the counter only from 180 code points', () => {
		mountDialog();
		type(title(), 'a'.repeat(179));
		expect(form().textContent).not.toContain('/ 200');
		type(title(), 'a'.repeat(183));
		expect(form().textContent).toContain('183 / 200');
		expect(title().getAttribute('aria-describedby')).toContain(form().querySelector('.counter')!.id);
	});

	it('words the duplicate check states as the spec does', async () => {
		vi.useFakeTimers();
		mountDialog();
		net.hold = true;
		type(title(), 'Board is slow');
		await vi.advanceTimersByTimeAsync(300);
		expect(form().querySelector('[aria-live]')!.textContent).toContain('Checking similar work…');
		net.held[0].resolve([]);
		await vi.advanceTimersByTimeAsync(10);
		expect(form().querySelector('[aria-live]')!.textContent?.trim()).toBe('Nothing similar found');
		type(title(), 'Board is slow now');
		await vi.advanceTimersByTimeAsync(300);
		net.held[1].resolve([hit('ABC-1', 'a'), hit('ABC-2', 'b')]);
		await vi.advanceTimersByTimeAsync(10);
		const live = form().querySelector('[aria-live]')!;
		expect(live.textContent?.trim()).toBe('2 similar items');
		expect(live.querySelector('[data-hit]')).toBeNull();
		expect(form().textContent).toContain('Already reported?');
		vi.useRealTimers();
		net.hold = false;
		net.similarFails = true;
		type(title(), 'Board is slow later');
		await vi.waitFor(() => expect(form().querySelector('[aria-live]')!.textContent?.trim()).toBe("Couldn't check similar work. You can still submit."));
	});

	it('shows the Reported from line with its link', () => {
		mountDialog({ path: '/epics/ABC-30', label: 'Payments', epic: null });
		const line = [...form().querySelectorAll('p')].find((p) => p.textContent?.startsWith('Reported from:'))!;
		expect(line.textContent?.trim()).toBe('Reported from: Payments');
		expect(line.querySelector('a')!.getAttribute('href')).toBe('/epics/ABC-30');
	});

	it('uses the new key for an unchanged retry after "Send anyway" is lost too, and stays uncertain after a refusal', async () => {
		const { oncreated } = mountDialog();
		type(title(), 'Maybe filed report');
		choose('Idea or change');
		net.post.push('throw');
		form().requestSubmit();
		await vi.waitFor(() => expect(report.draft?.uncertain).toBe(true));
		type(title(), 'Maybe filed report, edited');
		form().requestSubmit();
		flushSync();
		net.post.push('throw');
		button('Send anyway')!.click();
		await vi.waitFor(() => expect(net.posts).toHaveLength(2));
		await vi.waitFor(() => expect(alertText()).toContain("couldn't confirm"));
		net.post.push('refuse');
		form().requestSubmit();
		await vi.waitFor(() => expect(net.posts).toHaveLength(3));
		await vi.waitFor(() => expect(alertText()).toBe(fullLabel({ reason: 'no_criteria' })));
		expect(report.draft?.uncertain).toBe(true);
		form().requestSubmit();
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(net.posts[1].idempotency_key).not.toBe(net.posts[0].idempotency_key);
		expect(net.posts[2].idempotency_key).toBe(net.posts[1].idempotency_key);
		expect(net.posts[3].idempotency_key).toBe(net.posts[1].idempotency_key);
	});

	it('reads a finished hit as "done, dropped" or "done, completed"', async () => {
		net.hits = [
			hit('ABC-1', 'one', { delivery: 'done', resolution: 'dropped' }),
			hit('ABC-2', 'two', { delivery: 'done', resolution: 'completed' }),
			hit('ABC-3', 'three', { delivery: 'in_progress' })
		];
		mountDialog();
		type(title(), 'Board is slow');
		await vi.waitFor(() => expect(form().querySelectorAll('[data-hit]')).toHaveLength(3));
		const text = (k: string) => form().querySelector(`[data-hit="${k}"]`)!.textContent!;
		expect(text('ABC-1')).toContain('done, dropped');
		expect(text('ABC-2')).toContain('done, completed');
		expect(text('ABC-3')).toContain('in progress');
	});

	it('focuses the title and clears leftFrom when the hit is gone from the answer or the check fails', async () => {
		vi.useFakeTimers();
		openReport(ctx);
		report.draft!.title = 'Board is slow';
		report.draft!.leftFrom = 'ABC-412';
		net.hold = true;
		app = mount(ReportDialog, { target: document.body, props: { oncreated: vi.fn(), onviewstory: vi.fn() } });
		flushSync();
		await vi.advanceTimersByTimeAsync(10);
		desc().focus();
		net.held[0].resolve([hit('ABC-9', 'another')]);
		await vi.advanceTimersByTimeAsync(10);
		flushSync();
		expect(document.activeElement).toBe(title());
		expect(report.draft?.leftFrom).toBeNull();

		report.draft!.leftFrom = 'ABC-412';
		report.open = false;
		flushSync();
		report.open = true;
		flushSync();
		await vi.advanceTimersByTimeAsync(10);
		desc().focus();
		net.held[1].fail();
		await vi.advanceTimersByTimeAsync(10);
		flushSync();
		expect(document.activeElement).toBe(title());
		expect(report.draft?.leftFrom).toBeNull();
	});

	it('counts the 200-character limit in code points, truncating a longer title', () => {
		mountDialog();
		expect(title().hasAttribute('maxlength')).toBe(false);
		type(title(), 'a'.repeat(150) + '😀'.repeat(100));
		expect([...title().value]).toHaveLength(200);
		expect([...report.draft!.title]).toHaveLength(200);
		expect(form().textContent).toContain('200 / 200');
		type(title(), '😀'.repeat(200));
		expect([...report.draft!.title]).toHaveLength(200);
	});

	it('disables the hit buttons while a send is in flight', async () => {
		net.hits = [hit('ABC-1', 'one')];
		mountDialog();
		type(title(), 'Board is slow');
		choose('Idea or change');
		await vi.waitFor(() => expect(form().querySelector('[data-hit]')).not.toBeNull());
		form().requestSubmit();
		flushSync();
		expect((form().querySelector('[data-hit]') as HTMLButtonElement).disabled).toBe(true);
	});

	it('leaves a different draft alone when the send it started finishes', async () => {
		const { oncreated } = mountDialog();
		type(title(), 'First report here');
		choose('Idea or change');
		form().requestSubmit();
		flushSync();
		discardDraft();
		openReport(ctx);
		flushSync();
		type(title(), 'Second report');
		await vi.waitFor(() => expect(oncreated).toHaveBeenCalledTimes(1));
		expect(report.draft?.title).toBe('Second report');
		expect(report.open).toBe(true);
	});

	it('asks for a title when it is only whitespace, and focuses it', () => {
		mountDialog();
		type(title(), '   ');
		choose('Idea or change');
		desc().focus();
		form().requestSubmit();
		flushSync();
		expect(alertText()).toBe('Give it a title.');
		expect(document.activeElement).toBe(title());
	});

	it('links the kind error to the fieldset and each radio to its help text', () => {
		mountDialog();
		const radioHelp = radio("Something's broken").getAttribute('aria-describedby')!;
		expect(document.getElementById(radioHelp)!.textContent).toContain('It worked before');
		type(title(), 'Needs a kind');
		form().requestSubmit();
		flushSync();
		const set = form().querySelector('fieldset')!;
		expect(set.getAttribute('aria-invalid')).toBe('true');
		expect(document.getElementById(set.getAttribute('aria-describedby')!)!.textContent).toContain('Choose what kind');
	});

	it('trims an over-long title at the caret, keeping the text after it', () => {
		mountDialog();
		type(title(), 'a'.repeat(199) + 'Z');
		title().value = 'x' + title().value;
		title().setSelectionRange(1, 1);
		title().dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		expect([...title().value]).toHaveLength(200);
		expect(title().value.endsWith('Z')).toBe(true);
		expect(title().value.startsWith('x')).toBe(false);
		expect(title().selectionStart).toBe(0);
	});

	it('links the kind error to each radio as well as the fieldset', () => {
		mountDialog();
		const fieldset = form().querySelector('fieldset')!;
		expect(fieldset.getAttribute('role')).toBe('radiogroup');
		expect(fieldset.getAttribute('aria-required')).toBe('true');
		type(title(), 'Needs a kind');
		form().requestSubmit();
		flushSync();
		const err = form().querySelector('[role=alert]')!.id;
		expect(radio('Idea or change').getAttribute('aria-describedby')).toContain(err);
		expect(radio('Idea or change').getAttribute('aria-describedby')).toMatch(/help-story/);
	});
});
