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

/** Markdown to sanitised HTML. Needs a DOM (a browser, or jsdom in tests): without one DOMPurify returns its input. */
export function renderMarkdown(md: string): string {
	const html = marked.parse(md, { async: false, gfm: true });
	return DOMPurify.sanitize(html, CONFIG);
}
