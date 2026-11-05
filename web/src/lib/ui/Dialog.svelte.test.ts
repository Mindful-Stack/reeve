// @vitest-environment jsdom
// web/src/lib/ui/Dialog.svelte.test.ts
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, expect, it, vi } from 'vitest';
import Dialog from './Dialog.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

it('opens without showModal (jsdom), names itself by its title, focuses the first field and cancels through onclose', () => {
	expect(HTMLDialogElement.prototype.showModal).toBeUndefined(); // the environment this fallback exists for
	const onclose = vi.fn();
	const children = createRawSnippet(() => ({ render: () => '<form aria-label="New epic"><input aria-label="Title"></form>' }));
	app = mount(Dialog, { target: document.body, props: { title: 'New epic', onclose, children } });
	flushSync();
	const d = document.body.querySelector('dialog')!;
	expect(d.hasAttribute('open')).toBe(true);
	expect(document.getElementById(d.getAttribute('aria-labelledby')!)!.textContent).toBe('New epic');
	expect(document.activeElement).toBe(d.querySelector('input'));
	expect(d.dispatchEvent(new Event('cancel', { cancelable: true }))).toBe(false); // prevented
	expect(d.hasAttribute('open')).toBe(true);
	expect(onclose).toHaveBeenCalledOnce();
});

it('returns focus to the opener on unmount', () => {
	const opener = document.createElement('button');
	document.body.append(opener);
	opener.focus();
	const children = createRawSnippet(() => ({ render: () => '<input aria-label="Title">' }));
	app = mount(Dialog, { target: document.body, props: { title: 'T', onclose: vi.fn(), children } });
	flushSync();
	expect(document.activeElement).not.toBe(opener);
	unmount(app);
	app = null;
	expect(document.activeElement).toBe(opener);
});

it('reports a native close once', () => {
	const onclose = vi.fn();
	const children = createRawSnippet(() => ({ render: () => '<input>' }));
	app = mount(Dialog, { target: document.body, props: { title: 'T', onclose, children } });
	flushSync();
	document.body.querySelector('dialog')!.dispatchEvent(new Event('close'));
	expect(onclose).toHaveBeenCalledOnce();
});

it('does not report again when its own close() fires a native close event', () => {
	const proto = HTMLDialogElement.prototype as { close?: () => void };
	const original = proto.close;
	proto.close = function (this: HTMLDialogElement) {
		this.removeAttribute('open');
		this.dispatchEvent(new Event('close'));
	};
	try {
		const onclose = vi.fn();
		const children = createRawSnippet(() => ({ render: () => '<input>' }));
		app = mount(Dialog, { target: document.body, props: { title: 'T', onclose, children } });
		flushSync();
		unmount(app);
		app = null;
		expect(onclose).not.toHaveBeenCalled();
	} finally {
		if (original) proto.close = original;
		else delete proto.close;
	}
});

it('closable={false} disables the close button and swallows Escape without reporting onclose', () => {
	const onclose = vi.fn();
	const children = createRawSnippet(() => ({ render: () => '<input>' }));
	app = mount(Dialog, { target: document.body, props: { title: 'T', onclose, closable: false, children } });
	flushSync();
	const d = document.body.querySelector('dialog')!;
	const close = d.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!;
	expect(close.disabled).toBe(true);
	close.click();
	expect(d.dispatchEvent(new Event('cancel', { cancelable: true }))).toBe(false); // prevented
	expect(d.hasAttribute('open')).toBe(true);
	expect(onclose).not.toHaveBeenCalled();
});

it('closable={false} shows the dialog again when the browser closes it without a request', () => {
	const proto = HTMLDialogElement.prototype as { showModal?: () => void };
	const showModal = vi.fn(function (this: HTMLDialogElement) {
		this.setAttribute('open', '');
	});
	proto.showModal = showModal;
	try {
		const onclose = vi.fn();
		const children = createRawSnippet(() => ({ render: () => '<input>' }));
		app = mount(Dialog, { target: document.body, props: { title: 'T', onclose, closable: false, children } });
		flushSync();
		expect(showModal).toHaveBeenCalledOnce();
		const d = document.body.querySelector('dialog')!;
		// Escape in a browser that ignores the prevented cancel: the dialog closes natively and focus leaves it.
		const outside = document.body.appendChild(document.createElement('button'));
		outside.focus();
		expect(document.activeElement).toBe(outside);
		d.removeAttribute('open');
		d.dispatchEvent(new Event('close'));
		expect(showModal).toHaveBeenCalledTimes(2);
		expect(d.hasAttribute('open')).toBe(true);
		expect(document.activeElement).toBe(d.querySelector('input')); // shown again, with focus back on its first field
		expect(onclose).not.toHaveBeenCalled();
	} finally {
		delete proto.showModal;
	}
});
