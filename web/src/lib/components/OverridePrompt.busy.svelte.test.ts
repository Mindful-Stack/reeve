// @vitest-environment jsdom
// While the override POST is in flight the dialog cannot be dismissed: ×, Cancel are disabled and Escape does not cancel.
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import OverridePrompt from './OverridePrompt.svelte';

let app: ReturnType<typeof mount> | null = null;
afterEach(() => {
	if (app) unmount(app);
	app = null;
	document.body.innerHTML = '';
});

describe('OverridePrompt', () => {
	it('cannot be dismissed while the override POST is pending', async () => {
		let release!: () => void;
		const post = new Promise<void>((resolve) => (release = resolve));
		const props = $state({ busy: false });
		const oncancel = vi.fn();
		const onconfirm = vi.fn(() => {
			props.busy = true;
			return post.then(() => (props.busy = false));
		});
		app = mount(OverridePrompt, {
			target: document.body,
			props: {
				storyKey: 'ABC-1',
				target: 'Done',
				reasons: [],
				get busy() {
					return props.busy;
				},
				onconfirm,
				oncancel
			}
		});
		const close = document.body.querySelector('button[aria-label="Close"]') as HTMLButtonElement;
		const area = document.body.querySelector('textarea') as HTMLTextAreaElement;
		area.value = 'because';
		area.dispatchEvent(new Event('input', { bubbles: true }));
		flushSync();
		(document.body.querySelector('button.primary') as HTMLButtonElement).click();
		flushSync();
		expect(onconfirm).toHaveBeenCalledWith('because');
		expect(close.disabled).toBe(true);
		const cancel = [...document.body.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Cancel') as HTMLButtonElement;
		expect(cancel.disabled).toBe(true);
		cancel.click();
		flushSync();
		expect(oncancel).not.toHaveBeenCalled();
		document.body.querySelector('dialog')!.dispatchEvent(new Event('cancel', { cancelable: true }));
		flushSync();
		expect(oncancel).not.toHaveBeenCalled();
		release();
		await post;
		flushSync();
		expect(close.disabled).toBe(false);
		expect(cancel.disabled).toBe(false);
	});
});
