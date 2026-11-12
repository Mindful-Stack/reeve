// @vitest-environment jsdom
// web/src/lib/ui/Dialog.svelte.test.ts
import { createRawSnippet, flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
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

describe('backdrop click', () => {
	const rect = { left: 100, top: 100, right: 300, bottom: 300, width: 200, height: 200, x: 100, y: 100, toJSON() {} };
	function open(closable = true) {
		const onclose = vi.fn();
		const children = createRawSnippet(() => ({ render: () => '<input aria-label="Title">' }));
		app = mount(Dialog, { target: document.body, props: { title: 'T', onclose, closable, children } });
		flushSync();
		const d = document.body.querySelector('dialog')!;
		d.getBoundingClientRect = () => rect as DOMRect;
		return { d, onclose };
	}
	const at = (type: string, x: number, y: number) => new MouseEvent(type, { bubbles: true, clientX: x, clientY: y });

	it('closes when pressed and released outside the dialog box', () => {
		const { d, onclose } = open();
		d.dispatchEvent(at('pointerdown', 10, 10));
		d.dispatchEvent(at('click', 10, 10));
		expect(onclose).toHaveBeenCalledOnce();
	});
	it('stays open for a click on its own padding (target is the dialog, but inside its box)', () => {
		const { d, onclose } = open();
		d.dispatchEvent(at('pointerdown', 105, 105));
		d.dispatchEvent(at('click', 105, 105));
		expect(onclose).not.toHaveBeenCalled();
	});
	it('stays open when a drag starts inside and ends on the backdrop', () => {
		const { d, onclose } = open();
		d.querySelector('input')!.dispatchEvent(at('pointerdown', 150, 150));
		d.dispatchEvent(at('click', 10, 10));
		expect(onclose).not.toHaveBeenCalled();
	});
	it('stays open for a click on a child at (0, 0) after a backdrop press (target guard)', () => {
		const { d, onclose } = open();
		d.dispatchEvent(at('pointerdown', 10, 10));
		d.querySelector('input')!.dispatchEvent(at('click', 0, 0));
		expect(onclose).not.toHaveBeenCalled();
	});
	it('stays open while not closable', () => {
		const { d, onclose } = open(false);
		d.dispatchEvent(at('pointerdown', 10, 10));
		d.dispatchEvent(at('click', 10, 10));
		expect(onclose).not.toHaveBeenCalled();
	});
});
