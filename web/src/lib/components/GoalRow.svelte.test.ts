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
		expect(line).toContain('1/6');
		expect(line).toContain('AB');
		expect(row().querySelector('.dot')!.getAttribute('aria-label')).toBe('confidence: at risk');
		expect(row().querySelector('.badge')!.getAttribute('title')).toBe('ana.berg@example.com');
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
		expect(text(row().querySelector('[data-testid="confidence"]'))).toBe('at risk');
		expect(detail).toContain('1 completed · 2 in flight (1 in review) · 3 todo');
		expect(row().querySelector('[data-testid="gates"]')).not.toBeNull();
		expect(row().querySelector('[data-testid="controls"]')).not.toBeNull();
	});

	it('a next-sprint goal has no confidence, counts or progress, and says since when it waits', () => {
		show({ progress: undefined, open: true, goal: goal({ id: 7, sprint_id: 2, created_at: '2026-11-03T12:00:00Z' }) });
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
		expect(tag.getAttribute('aria-label')).toBe('Initiative ABC-5, Later, not in Now');
		expect(text(row().querySelector('.detail'))).toContain('ABC-5 Initiative ABC-5 · Later (not in Now)');
	});
});
