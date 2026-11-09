// @vitest-environment jsdom
// The urgent line counts "N d ago" from the `now` it is given, and separates its parts.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { UrgentMark } from '#lib/api/types.ts';
import UrgentLine from './UrgentLine.svelte';

vi.mock('$app/state', async () => {
	const { fakePage } = await import('#lib/test/net.ts');
	return { page: fakePage({ url: '/epics/ABC-5', params: { key: 'ABC-5' } }).page };
});

const mark: UrgentMark = { at: '2026-11-01T00:00:00Z', by: 'dan@example.com', reason: null, expires_at: '2099-01-01T00:00:00Z', active: true, done: false };
let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('UrgentLine', () => {
	it('re-renders its "N d ago" text when given a later now', () => {
		const props = $state({ now: new Date('2026-11-03T00:00:00Z') });
		app = mount(UrgentLine, {
			target: document.body,
			props: {
				itemKey: 'ABC-5',
				mark,
				sources: [{ key: 'ABC-5', item: 'epic' }, { key: 'ABC-9', item: 'story' }],
				titleOf: () => undefined,
				get now() {
					return props.now;
				},
				onremark: () => {},
				onclear: () => {}
			}
		});
		flushSync();
		const line = () => document.body.querySelector('[data-testid="urgent-line"]')!;
		expect(line().textContent).toContain('2 d ago');
		expect(line().querySelector('span[aria-hidden="true"]')?.textContent).toBe('·');
		props.now = new Date('2026-11-06T00:00:00Z');
		flushSync();
		expect(line().textContent).toContain('5 d ago');
	});
});
