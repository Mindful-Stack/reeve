// @vitest-environment jsdom
// The queue's entry adds the next step, when the decision is needed and the work it holds to a row; the title opens the drawer.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { RequiredDecision } from '#lib/api/types.ts';
import { blockedStory, decision as decisionDto, queueEntry } from '#lib/test/fixtures.ts';
import { resetDecisionDraftsForTests } from '#lib/decisionDrafts.svelte.ts';
import DecisionList from './DecisionList.svelte';

// The decisions page, scrolled to a lane: a story link opens the drawer over it and keeps the lane's hash.
vi.mock('$app/state', async () => ({
	page: (await import('#lib/test/net.ts')).fakePage({ url: '/decisions#next-sprint' }).page
}));

const net = vi.hoisted(() => ({ calls: [] as { method: string; path: string }[] }));
// The client captures `fetch` when it is created, so the fake goes in before the real module loads (#lib/test/net.ts).
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(({ method, path }) => {
		net.calls.push({ method, path });
		if (method === 'GET' && path.endsWith('/comments')) return [];
		throw new Error(`unexpected ${method} ${path}`);
	});
	return orig();
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
	net.calls.length = 0;
	resetDecisionDraftsForTests();
});

function decision(pr_url: string): RequiredDecision {
	return decisionDto({
		id: 1,
		on: 'RV-1',
		title: 'Pick a queue',
		decider: 'dan@example.com',
		check: { kind: 'proposed_pr_open' },
		record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1' },
		pr_url
	});
}

// The queue's entry adds the next step, when the decision is needed and the work it holds; the title opens the drawer.
describe('DecisionList entries', () => {
	const PR = 'https://github.com/x/y/pull/9';
	function mountWith(
		d: RequiredDecision,
		extra: Record<string, unknown>
	): { row: HTMLElement; refetch: (next: RequiredDecision[]) => void } {
		const props = $state({ decisions: [d], onchanged: async () => {}, ...extra });
		app = mount(DecisionList, { target: document.body, props: props as never });
		flushSync();
		return {
			row: document.body.querySelector('.decisions > li') as HTMLElement,
			refetch: (next) => {
				props.decisions = next;
				flushSync();
			}
		};
	}
	const entryFor = (d: RequiredDecision, over: Parameters<typeof queueEntry>[0]) => new Map([[d.id, queueEntry({ decision: d, ...over })]]);

	it('links the item in discovery to its initiative or epic page, by its level', () => {
		for (const [level, href] of [
			['initiative', '/initiatives/GRA-12'],
			['epic', '/epics/GRA-12']
		] as const) {
			const d = decision(PR);
			const { row } = mountWith(d, {
				entries: entryFor(d, { needed_in: 'next_sprint', needed_in_reason: { kind: 'discovery', key: 'GRA-12', level } })
			});
			expect(row.querySelector('.needed a')?.getAttribute('href')).toBe(href);
			expect(row.querySelector('.needed')?.textContent).toBe('Needed next sprint \u00b7 GRA-12 is in discovery');
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});

	it('links the promised item: a story opens its drawer over the lane it is in, an epic its page', () => {
		for (const [level, key, href, reset] of [
			['story', 'ABC-12', '/decisions?story=ABC-12#next-sprint', 'false'],
			['epic', 'ABC-5', '/epics/ABC-5', null]
		] as const) {
			const d = decision(PR);
			const { row } = mountWith(d, {
				entries: entryFor(d, {
					needed_in: 'next_sprint',
					needed_in_reason: { kind: 'promise', key, level, on: '2026-12-01', overdue: false }
				})
			});
			const a = row.querySelector('.needed a');
			expect(a?.getAttribute('href')).toBe(href);
			expect(a?.getAttribute('data-sveltekit-reset')).toBe(reset);
			expect(row.querySelector('.needed')?.textContent).toBe(`Needed next sprint \u00b7 promised by 1 Dec: ${key}`);
			unmount(app!);
			app = null;
			document.body.innerHTML = '';
		}
	});

	it('shows the next step as a link to the linked PR, the sprint it is needed in and what it holds', () => {
		const d = decision(PR);
		const { row } = mountWith(d, {
			entries: entryFor(d, {
				next_step: { kind: 'review_record', pr_url: PR },
				needed_in: 'next_sprint',
				needed_in_reason: { kind: 'goal', goal_id: 3, outcome: 'Ship tiers' },
				blocks: [blockedStory({ key: 'ABC-31' }), blockedStory({ key: 'ABC-32' })],
				under_way: [blockedStory({ key: 'ABC-33', delivery: 'in_progress' })]
			})
		});
		const step = row.querySelector('.next-step')!;
		expect(step.textContent?.trim()).toBe('Linked PR \u00b7 not accepted yet');
		expect(step.querySelector('a')?.getAttribute('href')).toBe(PR);
		// The label stays beside the reason, so a row in the review lane still says which sprint it is for.
		expect(row.querySelector('.needed')?.textContent).toBe('Needed next sprint \u00b7 for goal: Ship tiers');
		expect(row.querySelector('[data-field="blocks"]')?.textContent).toContain('Holds 2 stories \u00b7 1 under way');
	});

	it('shows no such lines for a row without an entry, and keeps its plain-text step when the PR url is unsafe', () => {
		const d = decision(PR);
		const { row } = mountWith(d, {});
		expect(row.querySelector('.next-step, .needed')).toBeNull();
		unmount(app!);
		document.body.innerHTML = '';
		const bad = decision('javascript:alert(1)');
		const second = mountWith(bad, { entries: entryFor(bad, { next_step: { kind: 'review_record', pr_url: 'javascript:alert(1)' } }) });
		expect(second.row.querySelector('.next-step')?.textContent?.trim()).toBe('Linked PR \u00b7 not accepted yet');
		expect(second.row.querySelector('.next-step a')).toBeNull();
	});

	it('links the PR of an unchecked decision too', () => {
		const d = { ...decision(PR), check: { kind: 'unchecked' as const } };
		const { row } = mountWith(d, {});
		expect(row.querySelector('.check a')?.getAttribute('href')).toBe(PR);
	});

	it('links the title to the drawer with a data-field for focus restore, only when given a decisionHref', () => {
		const d = decision(PR);
		const { row } = mountWith(d, { decisionHref: (id: number) => `/decisions?decision=${id}` });
		const link = row.querySelector<HTMLAnchorElement>('.title');
		expect(link?.tagName).toBe('A');
		expect(link?.getAttribute('href')).toBe('/decisions?decision=1');
		expect(link?.getAttribute('data-field')).toBe('open');
		// SvelteKit 3: a drawer opens over the view, which must not scroll or lose focus.
		expect(link?.getAttribute('data-sveltekit-reset')).toBe('false');
		unmount(app!);
		document.body.innerHTML = '';
		expect(mountWith(d, {}).row.querySelector('.title')?.tagName).toBe('SPAN');
	});

	it('links the record in its discussion through browse_url', () => {
		const d = { ...decision(PR), record: { repo: 'kb', path: 'docs/adr/1.md', record_id: 'kb/ADR-1', browse_url: 'https://github.com/o/kb/blob/HEAD/docs/adr/1.md' } };
		const { row } = mountWith(d, { expandAll: true });
		expect(row.querySelector('.record-line a.record')?.getAttribute('href')).toBe('https://github.com/o/kb/blob/HEAD/docs/adr/1.md');
	});

	it('expandAll opens each discussion once and loads its thread, and a discussion the user collapsed stays collapsed through a refetch', async () => {
		const d = decision(PR);
		const { row, refetch } = mountWith(d, { expandAll: true });
		const toggle = () => row.querySelector<HTMLButtonElement>('button[data-field="discussion"]')!;
		await vi.waitFor(() => expect(net.calls.some((c) => c.method === 'GET' && c.path === '/api/v1/decisions/1/comments')).toBe(true));
		expect(toggle().getAttribute('aria-expanded')).toBe('true');
		toggle().click();
		flushSync();
		expect(toggle().getAttribute('aria-expanded')).toBe('false');
		refetch([{ ...d, version: 2 }]);
		expect(toggle().getAttribute('aria-expanded')).toBe('false');
		// A decision that arrives later is opened, once.
		refetch([{ ...d, version: 2 }, decisionDto({ id: 2, title: 'Second', on: 'RV-2' })]);
		const second = document.body.querySelectorAll<HTMLButtonElement>('button[data-field="discussion"]')[1];
		expect(second.getAttribute('aria-expanded')).toBe('true');
	});
});
