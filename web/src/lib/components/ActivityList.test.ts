// @vitest-environment jsdom
// An item's activity words a promise from its payload, without the key (the list is about that one item).
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { activity } from '#lib/test/fixtures.ts';
import ActivityList from './ActivityList.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('ActivityList', () => {
	it('words promises and attachments from their payloads and leaves other actions as they are', () => {
		app = mount(ActivityList, {
			target: document.body,
			props: {
				entries: [
					activity({ id: 1, action: 'promised', payload: { item_type: 'story', on: '2026-11-20', to: 'Acme', previous: null } }),
					activity({ id: 2, action: 'promised', payload: { item_type: 'story', on: '2026-11-20', to: null, previous: { on: '2026-11-20', to: 'Acme' } } }),
					activity({ id: 3, action: 'promise_cleared', payload: { item_type: 'story', previous: { on: '2026-11-20', to: null } } }),
					activity({ id: 4, action: 'moved' }),
					activity({ id: 5, action: 'attachment_added', payload: { attachment_id: 'x', filename: 'shot.png' } })
				]
			}
		});
		flushSync();
		expect([...document.body.querySelectorAll('.action')].map((e) => e.textContent)).toEqual([
			'added attachment shot.png',
			'moved',
			'cleared the promise',
			"removed the promise's recipient",
			'promised by 20 Nov to Acme'
		]);
	});
});
