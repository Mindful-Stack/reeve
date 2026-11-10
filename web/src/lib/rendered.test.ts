// @vitest-environment jsdom
// Rendered Markdown containers swap a broken or deleted attachment image for "image unavailable" (#lib/rendered.ts).
import { afterEach, describe, expect, it } from 'vitest';
import { rendered } from './rendered.ts';
import { renderMarkdown } from './markdown.ts';

const id = '0f8b2c3e-1a2b-4c3d-8e9f-0a1b2c3d4e5f';
let detach: () => void = () => {};
afterEach(() => {
	detach();
	document.body.innerHTML = '';
});

function mountBox(md: string): HTMLElement {
	const box = document.createElement('div');
	box.innerHTML = renderMarkdown(md);
	document.body.append(box);
	detach = rendered()(box) as () => void;
	return box;
}

describe('rendered', () => {
	it('turns an image that is already shown into a chip when it is deleted', () => {
		const box = mountBox(`Before ![a](/api/v1/attachments/${id}) after ![b](https://example.com/b.png)`);
		window.dispatchEvent(new CustomEvent('reeve:attachment-removed', { detail: { id } }));
		expect(box.querySelector(`img[src$="${id}"]`)).toBeNull();
		expect(box.querySelector('.image-unavailable')?.textContent).toBe('image unavailable');
		// Its wrapping link goes with it; another image stays.
		expect(box.querySelector('a')).toBeNull();
		expect(box.querySelector('img[src="https://example.com/b.png"]')).not.toBeNull();
	});

	it('hears a failed load, which does not bubble, through capture', () => {
		const box = mountBox(`![a](/api/v1/attachments/${id})`);
		box.querySelector('img')!.dispatchEvent(new Event('error'));
		expect(box.querySelector('.image-unavailable')).not.toBeNull();
	});

	it('ignores an external image that fails', () => {
		const box = mountBox('![b](https://example.com/b.png)');
		box.querySelector('img')!.dispatchEvent(new Event('error'));
		expect(box.querySelector('.image-unavailable')).toBeNull();
	});
});
