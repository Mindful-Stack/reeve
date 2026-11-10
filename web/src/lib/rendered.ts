// web/src/lib/rendered.ts: for containers of rendered Markdown (Brief, CommentBody). An attachment image that fails to
// load, or is deleted while shown (`reeve:attachment-removed`, from #lib/components/Attachments.svelte), becomes a
// muted "image unavailable" chip; an image already painted would otherwise stay on screen after its delete.
import type { Attachment } from 'svelte/attachments';
import { ATTACHMENT_SRC } from '#lib/markdown.ts';

function unavailable(img: HTMLImageElement): void {
	const chip = img.ownerDocument.createElement('span');
	chip.className = 'image-unavailable';
	chip.textContent = 'image unavailable';
	// Which image it was: its alt text, for anyone pointing at the chip.
	if (img.alt) chip.title = img.alt;
	// The link the renderer wrapped it in goes too: it would open the same missing image.
	const link = img.parentElement?.tagName === 'A' && img.parentElement.childNodes.length === 1 ? img.parentElement : img;
	link.replaceWith(chip);
}

export function rendered(): Attachment<HTMLElement> {
	return (el) => {
		// `error` does not bubble: only a capturing listener on the container hears it.
		const onError = (e: Event) => {
			const t = e.target;
			if (t instanceof HTMLImageElement && ATTACHMENT_SRC.test(t.getAttribute('src') ?? '')) unavailable(t);
		};
		const onRemoved = (e: Event) => {
			const id = (e as CustomEvent<{ id: string }>).detail?.id;
			// An attachment id is a UUID; anything else names no image here (and needs no escaping in the selector).
			if (!id || !/^[0-9a-f-]{36}$/.test(id)) return;
			for (const img of el.querySelectorAll<HTMLImageElement>(`img[src="/api/v1/attachments/${id}"]`)) unavailable(img);
		};
		el.addEventListener('error', onError, true);
		window.addEventListener('reeve:attachment-removed', onRemoved);
		return () => {
			el.removeEventListener('error', onError, true);
			window.removeEventListener('reeve:attachment-removed', onRemoved);
		};
	};
}
