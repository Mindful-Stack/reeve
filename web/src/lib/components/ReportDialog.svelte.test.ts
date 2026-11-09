// @vitest-environment jsdom
// The Report dialog: the type is required, a template survives edits, the epic chip can be dropped, a double submit and a
// lost answer never file twice, the duplicate check drops stale answers and survives failing, and leaving to look at a
// hit or pressing Escape keeps the draft.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SimilarStory, Story } from '#lib/api/types.ts';
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
			if (mode === 'refuse') return json({ outcome: 'refused', reasons: [], overridable: false });
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
	it('focuses the title and refuses a submit with no kind', async () => {
		mountDialog();
		await vi.waitFor(() => expect(document.activeElement).toBe(title()));
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
});
