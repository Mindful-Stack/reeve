// @vitest-environment jsdom
// The image upload controller and its textarea attachment (spec: Screenshots on items, §3).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
const flushEffects = () => flushSync();
import type { Call } from './test/net';

const net = vi.hoisted(() => ({
	calls: [] as Call[],
	next: [] as Array<(c: Call) => unknown>,
	/** Every held upload, released when its test ends so none stays busy into the next. */
	held: [] as Array<() => void>
}));
vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch, failure } = await import('./test/net');
	installFetch(async (c) => {
		net.calls.push(c);
		const answer = net.next.shift();
		if (answer) return answer(c);
		if (c.path === '/api/v1/attachments') return failure('unexpected', 500);
		return {};
	});
	return orig();
});
const { ImageUploads, attachable, nameFor, altFor, stripOrphans, unique, MESSAGES } = await import('./attach.svelte.ts');
const { pasteEvent, dragEvent, file } = await import('./test/events');
const { ok, failure } = await import('./test/net');

/** An answer for one upload, released when the test says so. */
function deferred(id: string) {
	let release!: () => void;
	const gate = new Promise<void>((r) => (release = r));
	net.held.push(release);
	net.next.push(async () => {
		await gate;
		return ok({ id, url: `/api/v1/attachments/${id}`, filename: 'x', content_type: 'image/png', byte_size: 4, uploaded_by: 'a@x', uploaded_at: '2026-11-10T09:00:00Z' });
	});
	return release;
}
/** Waits until `cond` holds, failing with a timeout rather than silently going on. */
async function until(cond: () => boolean) {
	await vi.waitFor(() => expect(cond()).toBe(true));
}

let el: HTMLTextAreaElement;
let model: string;
let detach: () => void;
let u: InstanceType<typeof ImageUploads>;

beforeEach(() => {
	net.calls = [];
	net.next = [];
	model = '';
	el = document.createElement('textarea');
	document.body.append(el);
	// The `value=`/`oninput` shape ReportDialog uses: the model follows input events only.
	el.addEventListener('input', () => (model = el.value));
	u = new ImageUploads(
		() => model,
		(t) => {
			model = t;
			el.value = t;
		}
	);
	detach = attachable(u)(el) as () => void;
});
afterEach(async () => {
	detach();
	el.remove();
	for (const r of net.held.splice(0)) r();
	await new Promise((r) => setTimeout(r, 20));
});

describe('ImageUploads', () => {
	it('writes a placeholder at the caret through an input event, then the image', async () => {
		el.value = model = 'Look:\n';
		el.selectionStart = el.selectionEnd = 6;
		const release = deferred('a1');
		u.add([file('shot.png')], el);
		expect(model).toMatch(/^Look:\n!\[Uploading shot… ‹u\d+›\]\(\)$/);
		expect(u.busy).toBe(true);
		release();
		await until(() => !u.busy);
		expect(model).toBe('Look:\n![shot](/api/v1/attachments/a1)');
		expect(u.ids()).toEqual(['a1']);
		expect(u.pending[0].state).toBe('added');
		expect(u.status).toBe('shot.png added');
		const up = net.calls[0];
		expect(up.url.searchParams.get('filename')).toBe('shot.png');
		expect(up.headers.get('content-type')).toBe('application/octet-stream');
		expect([...up.bytes!]).toEqual([0x89, 0x50, 0x4e, 0x47]);
	});

	it('keeps two same-named files apart when they finish out of order', async () => {
		vi.useFakeTimers({ toFake: ['Date'] });
		vi.setSystemTime(new Date(2026, 10, 10, 14, 32, 5));
		const first = deferred('one');
		const second = deferred('two');
		u.add([file('image.png'), file('image.png')], el);
		const names = u.pending.map((p) => p.name);
		expect(names[0]).toMatch(/^screenshot-\d{4}-\d{2}-\d{2}-\d{6}\.png$/);
		// Two clipboard images in the same second keep apart names.
		expect(names[1]).toBe(names[0].replace('.png', '-2.png'));
		second();
		await until(() => model.includes('/two)'));
		first();
		await until(() => !u.busy);
		vi.useRealTimers();
		const lines = model.split('\n');
		expect(lines[0]).toContain('(/api/v1/attachments/one)');
		expect(lines[1]).toContain('(/api/v1/attachments/two)');
	});

	it('removes the placeholder and shows the server message when an upload fails', async () => {
		net.next.push(() => failure('the file is larger than 10 MiB', 413, 'validation'));
		u.add([file('big.png')], el);
		await until(() => !u.busy);
		expect(model).toBe('');
		expect(u.pending[0]).toMatchObject({ state: 'failed', error: 'the file is larger than 10 MiB' });
	});

	it('puts nothing back when the placeholder was edited away, and leaves the upload alone', async () => {
		const release = deferred('gone');
		u.add([file('shot.png')], el);
		el.value = model = 'rewritten';
		release();
		await until(() => !u.busy);
		expect(model).toBe('rewritten');
		expect(u.ids()).toEqual([]);
		expect(net.calls.filter((c) => c.method === 'DELETE')).toEqual([]);
	});

	it('sends only the uploads still shown, known ones included', async () => {
		const known = new ImageUploads(() => model, (t) => (model = t), [{ id: 'old', url: '/api/v1/attachments/old' }]);
		model = '![a](/api/v1/attachments/old)';
		expect(known.ids()).toEqual(['old']);
		model = 'nothing';
		expect(known.ids()).toEqual([]);
	});

	it('refuses a wrong type or size without uploading, but lets an untyped file through to the server', async () => {
		u.add([file('notes.txt', 'text/plain'), new File([new Uint8Array(10_485_761)], 'huge.png', { type: 'image/png' })], el);
		expect(u.pending.map((p) => p.error)).toEqual([MESSAGES.type, MESSAGES.size]);
		expect(net.calls).toEqual([]);
		expect(model).toBe('');
		const release = deferred('untyped');
		u.add([file('photo', '')], el);
		release();
		await until(() => !u.busy);
		expect(u.ids()).toEqual(['untyped']);
	});

	it('takes a removed image out of the text, and focuses nothing it should not', async () => {
		const release = deferred('rm');
		u.add([file('shot.png')], el);
		release();
		await until(() => !u.busy);
		expect(model).toBe('![shot](/api/v1/attachments/rm)');
		u.remove(u.pending[0]);
		expect(model).toBe('');
		expect(u.ids()).toEqual([]);
		expect(net.calls.some((c) => c.method === 'DELETE')).toBe(false);
	});

	it('claims an image moved here from another field', async () => {
		const release = deferred('moved');
		u.add([file('shot.png')], el);
		release();
		await until(() => !u.busy);
		let other = '';
		const elsewhere = new ImageUploads(() => other, (t) => (other = t));
		other = model;
		expect(elsewhere.ids()).toEqual(['moved']);
	});

	it('ends an insertion on a line of its own', () => {
		deferred('mid');
		el.value = model = 'foobar';
		el.selectionStart = el.selectionEnd = 3;
		u.add([file('shot.png')], el);
		expect(model).toMatch(/^foo\n!\[Uploading shot… ‹u\d+›\]\(\)\nbar$/);
	});

	it('forgets everything on reset and never deletes', async () => {
		const release = deferred('r');
		u.add([file('shot.png')], el);
		release();
		await until(() => !u.busy);
		u.reset();
		expect([u.pending, u.refs()]).toEqual([[], []]);
		expect(net.calls.some((c) => c.method === 'DELETE')).toBe(false);
	});

	it('tells a draft about every stored upload', async () => {
		const seen: unknown[] = [];
		const d = new ImageUploads(() => model, (t) => (model = t), [], (refs) => seen.push(refs));
		const release = deferred('d1');
		d.add([file('a.png')]);
		release();
		await until(() => !d.busy);
		expect(seen).toEqual([[{ id: 'd1', url: '/api/v1/attachments/d1' }]]);
	});
});

describe('ImageUploads, more', () => {
	it('keeps busy live when it was created in an effect that is gone (the Report dialog closing)', async () => {
		let made: InstanceType<typeof ImageUploads> | null = null;
		let text = '';
		const stop = $effect.root(() => {
			$effect(() => {
				made = new ImageUploads(() => text, (t) => (text = t));
			});
		});
		flushEffects();
		stop();
		const release = deferred('late');
		made!.add([file('shot.png')]);
		expect(made!.busy).toBe(true);
		release();
		await until(() => !made!.busy);
		expect(text).toBe('![shot](/api/v1/attachments/late)');
	});

	it('swaps the placeholder in place, leaving the caret where the user is typing', async () => {
		const release = deferred('caret');
		u.add([file('shot.png')], el);
		el.value += '\nmore text';
		el.dispatchEvent(new Event('input'));
		el.selectionStart = el.selectionEnd = el.value.length - 4;
		release();
		await until(() => !u.busy);
		expect(model).toBe('![shot](/api/v1/attachments/caret)\nmore text');
		// A whole-value write would have put the caret at the end.
		expect(el.value.slice(el.selectionStart)).toBe('text');
	});

	it('takes a placeholder out when its row is removed or reset mid-upload, and stores nothing for it', async () => {
		const seen: unknown[] = [];
		const c = new ImageUploads(() => model, (t) => (model = t), [], (r) => seen.push(r));
		const release = deferred('gone');
		c.add([file('shot.png')]);
		c.remove(c.pending[0]);
		expect(model).toBe('');
		release();
		await vi.waitFor(() => expect(net.calls.length).toBeGreaterThan(0));
		await new Promise((r) => setTimeout(r, 10));
		expect([c.ids(), seen, model]).toEqual([[], [], '']);
		const again = deferred('reset');
		c.add([file('two.png')]);
		c.reset();
		expect(model).toBe('');
		again();
	});

	it('says a lost upload in the app’s words', async () => {
		net.next.push(() => {
			throw new Error('network down');
		});
		u.add([file('shot.png')], el);
		await until(() => !u.busy);
		expect(u.pending[0].error).toBe('the image could not be uploaded; add it again');
		expect(model).toBe('');
	});

	it('numbers names that are taken', () => {
		expect(unique('a.png', new Set(['a.png', 'a-2.png']))).toBe('a-3.png');
		expect(unique('notes', new Set(['notes']))).toBe('notes-2');
		expect(unique('.hidden', new Set(['.hidden']))).toBe('.hidden-2');
	});

	it('warns before leaving only while an upload runs, even with its field gone', async () => {
		const release = deferred('leave');
		u.add([file('shot.png')], el);
		detach();
		detach = () => {};
		const during = new Event('beforeunload', { cancelable: true });
		window.dispatchEvent(during);
		expect(during.defaultPrevented).toBe(true);
		release();
		await until(() => !u.busy);
		const after = new Event('beforeunload', { cancelable: true });
		window.dispatchEvent(after);
		expect(after.defaultPrevented).toBe(false);
	});
});

describe('attachable', () => {
	it('keeps the window guard while any field is mounted, and only for files', () => {
		const second = document.createElement('textarea');
		document.body.append(second);
		const detachSecond = attachable(u)(second) as () => void;
		detach();
		detach = () => {};
		const beside = dragEvent('drop', [file('a.png')]);
		window.dispatchEvent(beside);
		expect(beside.defaultPrevented).toBe(true);
		const text = dragEvent('dragover', []);
		window.dispatchEvent(text);
		expect(text.defaultPrevented).toBe(false);
		detachSecond();
		const after = dragEvent('drop', [file('a.png')]);
		window.dispatchEvent(after);
		expect(after.defaultPrevented).toBe(false);
		second.remove();
	});

	it('handles an image-only paste and a copied file, and leaves a rich text paste to the browser', () => {
		deferred('p');
		const image = pasteEvent([file('image.png')]);
		el.dispatchEvent(image);
		expect(image.defaultPrevented).toBe(true);
		expect(u.pending).toHaveLength(1);
		const rich = pasteEvent([file('image.png')], { text: true, html: true });
		el.dispatchEvent(rich);
		expect(rich.defaultPrevented).toBe(false);
		expect(u.pending).toHaveLength(1);
		// A file copied in a file manager brings its name as plain text, and no HTML: it is the image.
		deferred('q');
		const copied = pasteEvent([file('shot.png')], { text: true });
		el.dispatchEvent(copied);
		expect(copied.defaultPrevented).toBe(true);
		expect(u.pending).toHaveLength(2);
	});

	it('takes a file drop, refusing a non-image with its message, and marks the drop target', () => {
		const over = dragEvent('dragover', [file('a.txt', 'text/plain')]);
		el.dispatchEvent(over);
		expect(over.defaultPrevented).toBe(true);
		expect(el.dataset.drop).toBe('on');
		const drop = dragEvent('drop', [file('a.txt', 'text/plain')]);
		el.dispatchEvent(drop);
		expect(drop.defaultPrevented).toBe(true);
		expect(el.dataset.drop).toBeUndefined();
		expect(u.pending[0].error).toBe(MESSAGES.type);
	});

	it('guards the window against a file dropped beside the field, until detached', () => {
		const beside = dragEvent('drop', [file('a.png')]);
		window.dispatchEvent(beside);
		expect(beside.defaultPrevented).toBe(true);
		detach();
		const after = dragEvent('drop', [file('a.png')]);
		window.dispatchEvent(after);
		expect(after.defaultPrevented).toBe(false);
		detach = () => {};
	});
});

describe('helpers', () => {
	it('names clipboard images by time and escapes alt text', () => {
		expect(nameFor(file('image.png'), new Date(2026, 10, 10, 14, 32, 5))).toBe('screenshot-2026-11-10-143205.png');
		expect(nameFor(file('mine.png'))).toBe('mine.png');
		expect(altFor('a[1]\\b.png')).toBe('a\\[1\\]\\\\b');
	});

	it('strips placeholders a reload left behind', () => {
		expect(stripOrphans('Before\n![Uploading x… ‹u7›]()\nAfter')).toBe('Before\nAfter');
	});
});
