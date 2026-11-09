// @vitest-environment jsdom
// The account side menu: Users only for admins, the current page marked.
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Layout from './+layout.svelte';

const route = vi.hoisted(() => ({ path: '/tokens', role: 'member' }));
vi.mock('$app/state', () => ({
	page: {
		get url() {
			return new URL(`http://localhost${route.path}`);
		}
	}
}));
vi.mock('#lib/me.svelte.ts', () => ({
	isAdmin: () => route.role === 'admin',
	session: {
		get me() {
			return { email: 'a@example.com', role: route.role };
		}
	}
}));

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	route.role = 'member';
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

function open(path: string) {
	route.path = path;
	const children = createRawSnippet(() => ({ render: () => '<p>content</p>' }));
	app = mount(Layout, { target: document.body, props: { children } as never });
	flushSync();
}
const links = () => [...document.body.querySelectorAll('nav[aria-label="Account"] a')] as HTMLAnchorElement[];

describe('account menu', () => {
	it('hides Users from non-admins', () => {
		open('/tokens');
		expect(links().map((a) => a.textContent)).toEqual(['Tokens', 'Activity']);
	});

	it('shows Users to admins', () => {
		route.role = 'admin';
		open('/tokens');
		expect(links().map((a) => a.textContent)).toEqual(['Tokens', 'Activity', 'Users', 'Access']);
	});

	it('marks only the active item', () => {
		open('/activity');
		const current = links().filter((a) => a.getAttribute('aria-current') === 'page');
		expect(current.map((a) => a.textContent)).toEqual(['Activity']);
	});
});
