import DOMPurify from 'dompurify';
import { marked } from 'marked';

// Every link opens in a new tab without a handle on this window, and a link whose scheme is not http(s), mailto or
// relative loses its href. Registered once per page load; it is global to DOMPurify, so all sanitising goes through here.
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
	if (node.tagName === 'A' && node.hasAttribute('href')) {
		const href = (node.getAttribute('href') ?? '').trim();
		if (/^(javascript|data|vbscript):/i.test(href.replace(/[\u0000- ]/g, ''))) {
			node.removeAttribute('href');
			return;
		}
		node.setAttribute('target', '_blank');
		node.setAttribute('rel', 'noopener noreferrer');
	}
});

const CONFIG = {
	// Anything that restyles the app or posts from it. `marked` passes raw HTML through, so this is the only barrier.
	FORBID_TAGS: ['style', 'form', 'input', 'button', 'textarea', 'select', 'iframe', 'object', 'embed', 'svg', 'math'],
	FORBID_ATTR: ['style', 'action', 'formaction'],
	ALLOW_DATA_ATTR: false,
	// http(s), mailto and relative URLs only.
	ALLOWED_URI_REGEXP: /^(?:https?:|mailto:|[^a-z]|[a-z+.-]+(?:[^a-z+.\-:]|$))/i
};

/** A same-origin attachment image (kb/ADR-0023: served by the server under the session's access check). */
export const ATTACHMENT_SRC = /^\/api\/v1\/attachments\/[0-9a-f-]{36}$/;

/** The attribute on the wrapper `renderMarkdown` puts around each table; select it as `[data-table-scroll]`. */
export const TABLE_SCROLL = 'data-table-scroll';

/** The attribute on a header cell that holds a word of LONG_WORD characters or more; select it as
 *  `th[data-long-word]`. On paper only such a header breaks anywhere (app.css): were every header to, a short one
 *  ("Step", over a column of numbers) would be squeezed to a letter a line whenever the table is wider than the page. */
export const LONG_WORD_CELL = 'data-long-word';

/** The length, in characters, from which a header's word counts as an identifier rather than an ordinary word. */
const LONG_WORD = 20;

/** Markdown to sanitised HTML. Needs a DOM (a browser, or jsdom in tests): without one DOMPurify returns its input.
 *  Images load lazily; an attachment image not already inside a link opens its full size in a new tab. */
export function renderMarkdown(md: string): string {
	const html = marked.parse(md, { async: false, gfm: true });
	const body = DOMPurify.sanitize(html, { ...CONFIG, RETURN_DOM: true }) as unknown as HTMLElement;
	if (typeof body?.querySelectorAll !== 'function') return String(body ?? '');
	for (const img of body.querySelectorAll('img')) {
		img.setAttribute('loading', 'lazy');
		img.setAttribute('decoding', 'async');
		const src = img.getAttribute('src') ?? '';
		if (ATTACHMENT_SRC.test(src) && !img.closest('a')) {
			const a = img.ownerDocument.createElement('a');
			a.setAttribute('href', src);
			a.setAttribute('target', '_blank');
			a.setAttribute('rel', 'noopener noreferrer');
			img.replaceWith(a);
			a.append(img);
		}
	}
	// A table gets a wrapper that scrolls sideways (styled in app.css); `rendered` (#lib/rendered.ts) makes the wrapper
	// a tab stop while it really scrolls, so the keyboard can scroll it. The wrapper is known by a data attribute set
	// here, after sanitising: an author can write a class, but ALLOW_DATA_ATTR keeps this marker out of their hands.
	for (const table of body.querySelectorAll('table')) {
		const wrap = table.ownerDocument.createElement('div');
		wrap.setAttribute(TABLE_SCROLL, '');
		table.replaceWith(wrap);
		wrap.append(table);
		// Set here for the same reason as the wrapper's marker: after sanitising, so it is never an author's.
		for (const th of table.querySelectorAll('th')) {
			if ((th.textContent ?? '').split(/\s+/).some((word) => [...word].length >= LONG_WORD)) th.setAttribute(LONG_WORD_CELL, '');
		}
	}
	return body.innerHTML;
}
