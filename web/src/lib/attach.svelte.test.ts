// @vitest-environment jsdom
// The image upload controller and its textarea attachment (spec: Screenshots on items, §3).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Call } from './test/net';

const net = vi.hoisted(() => ({
	calls: [] as Call[],
	next: [] as Array<(c: Call) => unknown>
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
const { ImageUploads, attachable, nameFor, altFor, stripOrphans, MESSAGES } = await import('./attach.svelte.ts');
const { pasteEvent, dragEvent, file } = await import('./test/events');
const { ok, failure } = await import('./test/net');

/** An answer for one upload, released when the test says so. */
function deferred(id: string) {
	let release!: () => void;
	const gate = new Promise<void>((r) => (release = r));
	net.next.push(async () => {
		await gate;
		return ok({ id, url: `/api/v1/attachments/${id}`, filename: 'x', content_type: 'image/png', byte_size: 4, uploaded_by: 'a@x', uploaded_at: '2026-11-10T09:00:00Z' });
	});
	return release;
}
const settle = () => new Promise((r) => setTimeout(r, 0));
async function until(cond: () => boolean) {
	for (let i = 0; i < 50 && !cond(); i++) await settle();
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
afterEach(() => {
	detach();
	el.remove();
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
		const first = deferred('one');
		const second = deferred('two');
		u.add([file('image.png'), file('image.png')], el);
		const names = u.pending.map((p) => p.name);
		expect(names[0]).toMatch(/^screenshot-\d{4}-\d{2}-\d{2}-\d{4}\.png$/);
		second();
		await until(() => model.includes('/two)'));
		first();
		await until(() => !u.busy);
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
		model = 'rewritten';
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

describe('attachable', () => {
	it('handles an image-only paste and leaves a text paste to the browser', () => {
		deferred('p');
		const image = pasteEvent([file('image.png')]);
		el.dispatchEvent(image);
		expect(image.defaultPrevented).toBe(true);
		expect(u.pending).toHaveLength(1);
		const rich = pasteEvent([file('image.png')], { text: true });
		el.dispatchEvent(rich);
		expect(rich.defaultPrevented).toBe(false);
		expect(u.pending).toHaveLength(1);
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
		expect(nameFor(file('image.png'), new Date(2026, 10, 10, 14, 32))).toBe('screenshot-2026-11-10-1432.png');
		expect(nameFor(file('mine.png'))).toBe('mine.png');
		expect(altFor('a[1]\\b.png')).toBe('a\\[1\\]\\\\b');
	});

	it('strips placeholders a reload left behind', () => {
		expect(stripOrphans('Before\n![Uploading x… ‹u7›]()\nAfter')).toBe('Before\nAfter');
	});
});
