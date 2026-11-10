// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { renderMarkdown } from './markdown.ts';

function parse(md: string): HTMLElement {
	const el = document.createElement('div');
	el.innerHTML = renderMarkdown(md);
	return el;
}

describe('renderMarkdown', () => {
	it('renders plain Markdown', () => {
		expect(parse('**bold**').querySelector('strong')?.textContent).toBe('bold');
	});

	it('keeps escaped entity text literal, as the Jira import writes it', () => {
		expect(parse('a &amp;lt;b&amp;gt; &amp; c').textContent?.trim()).toBe('a &lt;b&gt; & c');
	});

	// The destinations below are what the Jira import writes for hostile hrefs (crates/reeve/src/jira/adf.rs,
	// `link_destination`): each must parse as one link whose href is the whole escaped URL, with nothing after it.
	it('keeps an escaped import link destination whole', () => {
		const one = (md: string) => {
			const el = parse(md);
			expect(el.querySelectorAll('a')).toHaveLength(1);
			expect(el.querySelector('img, b')).toBeNull();
			expect(el.textContent?.trim()).toBe('click');
			return el.querySelector('a')!;
		};
		expect(one('[click](https://a.example/x\\)%20%3Cimg%20src=x%20onerror=alert\\(1\\)%3E)').getAttribute('href')).toBe(
			'https://a.example/x)%20%3Cimg%20src=x%20onerror=alert(1)%3E'
		);
		const quoted = one('[click](https://a.example/%22onmouseover=%22alert\\(1\\))');
		expect(quoted.getAttribute('href')).toBe('https://a.example/%22onmouseover=%22alert(1)');
		expect(quoted.hasAttribute('onmouseover')).toBe(false);
		expect(one('[click](https://a.example/a%0A%0A%3Cb%3Ex%3C/b%3E\\\\)').getAttribute('href')).toBe(
			'https://a.example/a%0A%0A%3Cb%3Ex%3C/b%3E%5C'
		);
		expect(one('[click](https://a.example/?a=1&amp;amp;b=2)').getAttribute('href')).toBe('https://a.example/?a=1&amp;b=2');
	});

	it('removes script elements', () => {
		const el = parse('hi <script>alert(1)</script>');
		expect(el.querySelector('script')).toBeNull();
		expect(el.innerHTML).not.toContain('alert');
	});

	it('removes inline event handlers', () => {
		const el = parse('<img src="/x.png" onerror="alert(1)"><p onclick="alert(1)">x</p>');
		expect(el.querySelector('[onerror]')).toBeNull();
		expect(el.querySelector('[onclick]')).toBeNull();
	});

	it('removes style attributes', () => {
		const el = parse('<div style="position:fixed;inset:0">overlay</div>');
		expect(el.querySelector('[style]')).toBeNull();
		expect(el.textContent).toContain('overlay');
	});

	it('removes style elements', () => {
		const el = parse('<style>body *{visibility:hidden}</style>text');
		expect(el.querySelector('style')).toBeNull();
		expect(el.innerHTML).not.toContain('visibility');
	});

	it('removes forms and their controls', () => {
		const el = parse('<form action="/api/v1/x" method="post"><input name="a"><button>go</button></form>');
		expect(el.querySelector('form, input, button')).toBeNull();
	});

	it('removes javascript: and data: hrefs', () => {
		const el = parse('[a](javascript:alert(1)) <a href="data:text/html,x">b</a> <a href=" JaVa\nscript:alert(1)">c</a>');
		for (const a of el.querySelectorAll('a')) expect(a.hasAttribute('href')).toBe(false);
	});

	it('opens https links in a new tab without opener', () => {
		const a = parse('[x](https://example.com/p)').querySelector('a');
		expect(a?.getAttribute('href')).toBe('https://example.com/p');
		expect(a?.getAttribute('target')).toBe('_blank');
		expect(a?.getAttribute('rel')).toBe('noopener noreferrer');
	});

	it('keeps relative and mailto links', () => {
		const el = parse('[a](/epics/E-1) [b](mailto:a@x.se)');
		expect([...el.querySelectorAll('a')].map((a) => a.getAttribute('href'))).toEqual(['/epics/E-1', 'mailto:a@x.se']);
	});
});

describe('images', () => {
	const id = '0f8b2c3e-1a2b-4c3d-8e9f-0a1b2c3d4e5f';
	it('loads lazily and links an attachment image to its full size', () => {
		const html = renderMarkdown(`![shot](/api/v1/attachments/${id})`);
		const box = document.createElement('div');
		box.innerHTML = html;
		const img = box.querySelector('img')!;
		expect(img.getAttribute('loading')).toBe('lazy');
		expect(img.getAttribute('decoding')).toBe('async');
		const a = img.closest('a')!;
		expect(a.getAttribute('href')).toBe(`/api/v1/attachments/${id}`);
		expect(a.getAttribute('target')).toBe('_blank');
	});

	it('leaves an image already inside a link alone, and does not wrap an external one', () => {
		const box = document.createElement('div');
		box.innerHTML = renderMarkdown(`[![shot](/api/v1/attachments/${id})](https://example.com)`);
		expect(box.querySelectorAll('a')).toHaveLength(1);
		expect(box.querySelector('a')!.getAttribute('href')).toBe('https://example.com');
		box.innerHTML = renderMarkdown('![x](https://example.com/x.png)');
		expect(box.querySelector('a')).toBeNull();
		expect(box.querySelector('img')!.getAttribute('loading')).toBe('lazy');
	});
});
