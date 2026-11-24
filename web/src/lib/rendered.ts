// web/src/lib/rendered.ts: for containers of rendered Markdown (Brief, CommentBody). An attachment image that fails to
// load, or is deleted while shown (`reeve:attachment-removed`, from #lib/components/Attachments.svelte), becomes a
// muted "image unavailable" chip; an image already painted would otherwise stay on screen after its delete.
// A table's wrapper (`[data-table-scroll]`, from `renderMarkdown`) is a named tab stop while the table is wider than it.
import type { Attachment } from 'svelte/attachments';
import { ATTACHMENT_SRC, TABLE_SCROLL } from '#lib/markdown.ts';

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

/** Only a wrapper that scrolls sideways is a tab stop (the arrow keys scroll it), and it says what it is; a table that
 *  fits gets no stop. Checked on mount, when the container, a wrapper or a table resizes (a late image or font widens
 *  a table without changing the container's box) and when the container's Markdown is replaced. */
function tableStops(el: HTMLElement): () => void {
	const wrappers = () => [...el.querySelectorAll(`[${TABLE_SCROLL}]`)];
	const check = () => {
		for (const wrap of wrappers()) {
			if (wrap.scrollWidth > wrap.clientWidth) {
				wrap.setAttribute('tabindex', '0');
				wrap.setAttribute('role', 'region');
				wrap.setAttribute('aria-label', 'Scrollable table');
			} else {
				for (const name of ['tabindex', 'role', 'aria-label']) wrap.removeAttribute(name);
			}
		}
	};
	// jsdom (the component tests) has no ResizeObserver; a browser does.
	const sizes = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(check);
	let watched = new Set<Element>();
	// The observer holds exactly the container, the wrappers and their tables that are there now.
	const watch = () => {
		const now = new Set<Element>([el]);
		for (const wrap of wrappers()) {
			now.add(wrap);
			if (wrap.firstElementChild) now.add(wrap.firstElementChild);
		}
		for (const gone of watched) if (!now.has(gone)) sizes?.unobserve(gone);
		for (const added of now) if (!watched.has(added)) sizes?.observe(added);
		watched = now;
	};
	check();
	watch();
	const nodes = new MutationObserver(() => {
		watch();
		check();
	});
	nodes.observe(el, { childList: true });
	return () => {
		sizes?.disconnect();
		nodes.disconnect();
		watched.clear();
	};
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
		const stops = tableStops(el);
		return () => {
			stops();
			el.removeEventListener('error', onError, true);
			window.removeEventListener('reeve:attachment-removed', onRemoved);
		};
	};
}
