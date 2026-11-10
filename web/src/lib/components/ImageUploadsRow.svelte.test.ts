// @vitest-environment jsdom
// The uploads row: Add image never submits, Remove gives focus back to the field (or Add image), and the status is
// announced politely.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('#lib/api/client.ts', async (orig) => {
	const { installFetch } = await import('#lib/test/net.ts');
	installFetch(() => new Promise(() => {})); // uploads never answer: the rows stay "Uploading…"
	return orig();
});
const { ImageUploads } = await import('#lib/attach.svelte.ts');
const { default: Row } = await import('./ImageUploadsRow.svelte');
const { file } = await import('#lib/test/events.ts');

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

function setup(disabledField = false) {
	let text = '';
	const field = document.createElement('textarea');
	field.disabled = disabledField;
	document.body.append(field);
	const uploads = new ImageUploads(() => text, (t) => (text = t));
	app = mount(Row, { target: document.body, props: { uploads, field } });
	uploads.add([file('shot.png')]);
	flushSync();
	return { field, uploads };
}

describe('ImageUploadsRow', () => {
	it('names Remove by its file, gives focus back to the field, and announces politely', () => {
		const { field } = setup();
		const live = document.body.querySelector('[aria-live=polite]')!;
		expect(live.textContent).toBe('Uploading shot.png…');
		const remove = document.body.querySelector<HTMLButtonElement>('button[aria-label="Remove shot.png"]')!;
		expect(remove.type).toBe('button');
		remove.click();
		flushSync();
		expect(document.activeElement).toBe(field);
		expect(document.body.querySelector('li')).toBeNull();
	});

	it('gives focus to Add image when the field is disabled', () => {
		setup(true);
		document.body.querySelector<HTMLButtonElement>('button[aria-label="Remove shot.png"]')!.click();
		flushSync();
		expect(document.activeElement?.textContent).toBe('Add image');
	});
});
