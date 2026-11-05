// @vitest-environment jsdom
// web/src/lib/ui/blocks.svelte.test.ts
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Avatar from './Avatar.svelte';
import Notice from './Notice.svelte';
import PageHeader from './PageHeader.svelte';
import StatusDot from './StatusDot.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});
const text = (t: string) => createRawSnippet(() => ({ render: () => `<span>${t}</span>` }));

describe('ui blocks', () => {
	it('StatusDot shows its word, never colour alone', () => {
		app = mount(StatusDot, {
			target: document.body,
			props: { tone: 'warn', label: 'no criteria' }
		});
		expect(document.body.textContent).toContain('no criteria');
		expect(document.body.querySelector('.dot.warn')).not.toBeNull();
	});
	it('Avatar shows initials from the email and the full email as its title', () => {
		app = mount(Avatar, {
			target: document.body,
			props: { email: 'anna.lind@example.com' }
		});
		const el = document.body.querySelector('.avatar')!;
		expect(el.textContent).toBe('AL');
		expect(el.getAttribute('title')).toBe('anna.lind@example.com');
	});
	it('Avatar says unowned without an email', () => {
		app = mount(Avatar, { target: document.body, props: { email: null } });
		expect(document.body.querySelector('.avatar')!.getAttribute('title')).toBe('unowned');
	});
	it('Notice is an alert for errors and offers Retry when given one', () => {
		const onretry = vi.fn();
		app = mount(Notice, {
			target: document.body,
			props: { tone: 'error', onretry, children: text('Could not load') }
		});
		const n = document.body.querySelector('[role="alert"]')!;
		expect(n.textContent).toContain('Could not load');
		(n.querySelector('button') as HTMLButtonElement).click();
		flushSync();
		expect(onretry).toHaveBeenCalledOnce();
	});
	it('StatusDot hides the dot from assistive tech', () => {
		app = mount(StatusDot, {
			target: document.body,
			props: { tone: 'ready', label: 'ok' }
		});
		expect(document.body.querySelector('.dot')!.getAttribute('aria-hidden')).toBe('true');
	});
	it.each([
		['anna@x', 'anna@x'],
		[null, 'unowned'],
		['', 'unowned']
	])('Avatar %s has accessible name %s', (email, name) => {
		app = mount(Avatar, { target: document.body, props: { email } });
		expect(document.body.querySelector('.avatar')!.getAttribute('aria-label')).toBe(name);
		expect(document.body.querySelector('.avatar')!.getAttribute('role')).toBe('img');
	});
	it.each([
		['daniel@x', 'DA'],
		['_anna@x', 'AN'],
		['__@x', '–'],
		['😀.x@x', '😀X'],
		['😀a@x', '😀A']
	])('Avatar initials for %s', (email, out) => {
		app = mount(Avatar, { target: document.body, props: { email } });
		expect(document.body.querySelector('.avatar')!.textContent).toBe(out);
	});
	it('Notice defaults to a status without buttons', () => {
		app = mount(Notice, {
			target: document.body,
			props: { children: text('Hi') }
		});
		expect(document.body.querySelector('[role="status"]')).not.toBeNull();
		expect(document.body.querySelectorAll('button').length).toBe(0);
	});
	it('PageHeader count: absent, zero, positive', () => {
		app = mount(PageHeader, {
			target: document.body,
			props: { title: 'Backlog' }
		});
		expect(document.body.querySelector('.count')).toBeNull();
		unmount(app);
		app = mount(PageHeader, {
			target: document.body,
			props: { title: 'Backlog', count: 0 }
		});
		expect(document.body.querySelector('h1')!.textContent).toBe('Backlog 0');
		unmount(app);
		app = mount(PageHeader, {
			target: document.body,
			props: { title: 'Backlog', count: 3 }
		});
		expect(document.body.querySelector('.count')!.textContent).toBe('3');
	});
});
