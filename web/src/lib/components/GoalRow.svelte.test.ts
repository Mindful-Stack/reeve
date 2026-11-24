// @vitest-environment jsdom
// A goal on the sprint page: one line when closed, its detail when open.
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { goal, goalGates, goalInitiative } from '#lib/test/fixtures.ts';
import GoalRow from './GoalRow.svelte';

vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	return { page: fakePage({ url: '/sprint' }).page };
});

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

const controls = createRawSnippet(() => ({ render: () => '<p data-testid="controls">controls</p>' }));
const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, ' ').trim() ?? '';
const row = () => document.querySelector('[data-testid="goal-7"]')!;

function show(props: Record<string, unknown> = {}) {
	const ontoggle = vi.fn();
	app = mount(GoalRow, {
		target: document.body,
		props: {
			goal: goal({ id: 7, outcome: 'Customers book', check: 'one books', notes: 'by phone', owner: 'ana.berg@example.com', links: ['ABC-2', 'ABC-3'], confidence: 'at_risk' }),
			gates: goalGates({ goal_id: 7, initiatives: [goalInitiative('ABC-1')], discoveries: [{ key: 'ABC-1', item: 'initiative', title: 'Bookings' }], discovery_total: 1 }),
			progress: { goal_id: 7, completed: 1, in_flight: 2, in_review: 1, todo: 3, blocked_decisions: 0 },
			firstBucket: 'Now',
			active: true,
			open: false,
			ontoggle,
			controls,
			...props
		}
	});
	flushSync();
	return ontoggle;
}

describe('a goal row', () => {
	it('is one line when closed: outcome, confidence, tag, what it waits on, links, progress and initials', () => {
		show();
		const line = text(row());
		expect(line).toContain('Customers book');
		expect(line).toContain('ABC-1 Now');
		expect(line).toContain('waits on 1');
		expect(line).toContain('2 links');
		expect(text(row().querySelector('.numbers'))).toBe('1/6 1 of 6 completed');
		expect(text(row().querySelector('.badge [aria-hidden]'))).toBe('AB');
		const bar = row().querySelector('progress')!;
		expect([bar.value, bar.max]).toEqual([1, 6]);
		expect(bar.getAttribute('aria-hidden')).toBe('true');
		const dot = row().querySelector('.dot')!;
		expect(dot.getAttribute('aria-label')).toBe('confidence: at risk');
		expect(dot.getAttribute('title')).toBe('confidence: at risk');
		expect(text(row().querySelector('.badge'))).toContain('Owner: ana.berg@example.com');
		expect(row().querySelector('.badge')!.getAttribute('title')).toBe('ana.berg@example.com');
		expect(row().getAttribute('aria-labelledby')).toBe('goal-7-outcome');
		expect(document.getElementById('goal-7-outcome')!.textContent).toBe('Customers book');
		// Nothing of the detail is in the page while closed.
		expect(line).not.toContain('one books');
		expect(line).not.toContain('by phone');
		expect(document.querySelector('[data-testid="controls"]')).toBeNull();
		expect(document.querySelector('[data-testid="gates"]')).toBeNull();
	});

	it('asks to be toggled from its disclosure button, which says whether it is open', () => {
		const ontoggle = show();
		const b = row().querySelector<HTMLButtonElement>('button.disclose')!;
		expect(b.getAttribute('aria-expanded')).toBe('false');
		b.click();
		expect(ontoggle).toHaveBeenCalledTimes(1);
	});

	it('shows its detail when open: initiatives, check, notes, confidence, counts, gates and the controls', () => {
		show({ open: true });
		const b = row().querySelector<HTMLButtonElement>('button.disclose')!;
		expect(b.getAttribute('aria-expanded')).toBe('true');
		expect(document.getElementById(b.getAttribute('aria-controls')!)).not.toBeNull();
		const detail = text(row().querySelector('.detail'));
		expect(detail).toContain('ABC-1 Initiative ABC-1 · Now');
		expect(detail).toContain("We'll know when: one books");
		expect(detail).toContain('by phone');
		expect(detail).toContain('Owner: ana.berg@example.com');
		expect(text(row().querySelector('[data-testid="confidence"]'))).toBe('at risk');
		expect(detail).toContain('1 completed · 2 in flight (1 in review) · 3 todo');
		expect(row().querySelector('[data-testid="gates"]')).not.toBeNull();
		expect(row().querySelector('[data-testid="controls"]')).not.toBeNull();
	});

	it('a next-sprint goal has no confidence, counts or progress, and says since when it waits', () => {
		show({ progress: undefined, active: false, open: true, goal: goal({ id: 7, sprint_id: 2, created_at: '2026-11-03T12:00:00Z' }) });
		expect(row().querySelector('.dot')).toBeNull();
		expect(row().querySelector('progress')).toBeNull();
		expect(row().querySelector('[data-testid="confidence"]')).toBeNull();
		expect(text(row())).toContain('Waiting since 3 Nov');
		expect(text(row())).not.toContain('completed');
	});

	it('marks the tag when an initiative is outside the first bucket, in words too', () => {
		show({ gates: goalGates({ goal_id: 7, initiatives: [goalInitiative('ABC-5', { horizon: 'Later', in_first_bucket: false })] }), open: true });
		const tag = row().querySelector('.tag')!;
		expect(tag.classList.contains('warn')).toBe(true);
		expect(tag.getAttribute('aria-label')).toBeNull();
		expect(tag.getAttribute('title')).toBe('Initiative ABC-5, Later, not in Now');
		expect(text(tag.querySelector('.visually-hidden'))).toBe('Initiative ABC-5, Later, not in Now');
		expect(text(tag)).toContain('ABC-5 Later !');
		expect(text(row().querySelector('.detail'))).toContain('ABC-5 Initiative ABC-5 · Later (not in Now)');
	});

	it('never warns without a configured bucket, with an initiative outside it or none', () => {
		show({ firstBucket: null, gates: goalGates({ goal_id: 7, initiatives: [goalInitiative('ABC-5', { horizon: 'Later', in_first_bucket: false })] }) });
		let tag = row().querySelector('.tag')!;
		expect(tag.classList.contains('warn')).toBe(false);
		expect(text(tag)).not.toContain('!');
		unmount(app!);
		document.body.innerHTML = '';
		show({ firstBucket: null, gates: goalGates({ goal_id: 7 }) });
		tag = row().querySelector('.tag')!;
		expect(tag.firstChild!.textContent).toBe('no initiative');
		expect(tag.classList.contains('warn')).toBe(false);
	});

	it('without gates says no initiative, waits on nothing and shows no gates block', () => {
		show({ gates: undefined, open: true });
		expect(text(row().querySelector('.tag'))).toContain('no initiative');
		expect(text(row())).not.toContain('waits on');
		expect(row().querySelector('[data-testid="gates"]')).toBeNull();
	});

	it('shows "waits on" only above zero', () => {
		show({ gates: goalGates({ goal_id: 7 }) });
		expect(text(row())).not.toContain('waits on');
	});

	it('renders the lead snippet before the disclosure', () => {
		const lead = createRawSnippet(() => ({ render: () => '<input type="checkbox" data-testid="lead" />' }));
		show({ lead });
		const l = row().querySelector('[data-testid="lead"]')!;
		const b = row().querySelector('button.disclose')!;
		expect(l.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
	});

	it('shows the confidence note and the suggestion when open', () => {
		show({
			open: true,
			goal: goal({ id: 7, confidence: 'at_risk', confidence_note: 'vendor late', suggested_confidence: 'off_track', suggested_by: 'ben@example.com', suggested_note: 'two slipped' })
		});
		const detail = text(row().querySelector('.detail'));
		expect(detail).toContain('(vendor late)');
		expect(text(row().querySelector('[data-testid="suggestion"]'))).toBe('suggested: off track by ben@example.com (two slipped)');
	});

	it('an active goal without a counts row is not drawn as a draft', () => {
		show({ progress: undefined, active: true, open: true });
		expect(row().classList.contains('draft')).toBe(false);
		expect(row().querySelector('.dot')).not.toBeNull();
		expect(row().querySelector('progress')).toBeNull();
		expect(text(row().querySelector('.detail'))).toContain('0 completed · 0 in flight (0 in review) · 0 todo');
		expect(text(row())).not.toContain('Waiting since');
	});
});
