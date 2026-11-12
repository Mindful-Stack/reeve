// The colour contract of the design: every pair a page renders must reach its WCAG ratio in both schemes.
// app.css is the single source; values are read from the marked token blocks.
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { contrast } from './color.ts';

const css = readFileSync(new URL('../app.css', import.meta.url), 'utf8');

function raw(marker: string): string {
	const start = css.indexOf(`/* tokens:${marker} */`);
	if (start === -1) throw new Error(`app.css has no /* tokens:${marker} */ block`);
	return css.slice(css.indexOf('{', start) + 1, css.indexOf('}', start)).replace(/\s+/g, ' ').trim();
}

function block(marker: string): Record<string, string> {
	const start = css.indexOf(`/* tokens:${marker} */`);
	if (start === -1) throw new Error(`app.css has no /* tokens:${marker} */ block`);
	const body = css.slice(css.indexOf('{', start) + 1, css.indexOf('}', start));
	return Object.fromEntries([...body.matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})\s*;/gi)].map((m) => [m[1], m[2]]));
}

const schemes = { light: block('light'), dark: block('dark') };

const AA = 4.5;
const UI = 3;
const pairs: [string, string, number][] = [
	['foreground', 'background', AA], ['foreground', 'raised', AA], ['foreground', 'lighter-background', AA],
	['muted', 'background', AA], ['muted', 'raised', AA], ['muted', 'lighter-background', AA],
	['accent', 'background', AA], ['on-accent', 'accent', AA], ['danger', 'background', AA],
	['ready', 'background', UI], ['ready', 'raised', UI], ['warn', 'background', UI], ['warn', 'raised', UI],
	['pr-open', 'background', UI], ['pr-merged', 'background', UI],
	['line-control', 'background', UI], ['line-control', 'raised', UI],
	// Promise chips (PromiseChip.svelte): text is --foreground on warn-soft, ready-soft, lighter-background (muted) and selection (neutral).
	['warn', 'warn-soft', UI], ['ready', 'ready-soft', UI], ['foreground', 'warn-soft', AA], ['foreground', 'ready-soft', AA], ['foreground', 'selection', AA],
	['pr-open', 'raised', UI], ['pr-merged', 'raised', UI],
	['danger', 'raised', AA], ['foreground', 'danger-soft', AA], ['danger', 'danger-soft', AA], ['muted', 'danger-soft', AA], ['accent', 'danger-soft', AA], ['line-control', 'danger-soft', UI],
	['danger', 'lighter-background', AA], ['accent', 'lighter-background', AA], ['line-control', 'lighter-background', UI]
];

describe.each(Object.entries(schemes))('%s scheme', (_, t) => {
	it.each(pairs)('%s on %s reaches %d:1', (fg, bg, min) => {
		expect(t[fg], `--${fg} missing`).toBeDefined();
		expect(t[bg], `--${bg} missing`).toBeDefined();
		expect(contrast(t[fg], t[bg])).toBeGreaterThanOrEqual(min);
	});
});

// A search hit's <mark> sits in the result link, so with `color: inherit` its text is the link's accent; whatever colour
// the mark rule gives its text, that text on --mark must reach AA in both schemes.
describe('search highlight', () => {
	const rule = css.slice(css.search(/^mark \{/m), css.indexOf('}', css.search(/^mark \{/m)));
	const color = /color:\s*([^;]+);/.exec(rule.replace(/background[^;]*;/, ''))?.[1].trim();
	// `inherit` takes the colour of where the mark sits: body text and the result link.
	const texts = !color || color === 'inherit' ? ['foreground', 'accent'] : [/^var\(--([a-z-]+)\)$/.exec(color)![1]];
	describe.each(Object.entries(schemes))('%s scheme', (_, t) => {
		it.each(texts)('%s on mark reaches 4.5:1', (fg) => {
			expect(t[fg], `--${fg} missing`).toBeDefined();
			expect(t.mark, '--mark missing').toBeDefined();
			expect(contrast(t[fg], t.mark)).toBeGreaterThanOrEqual(AA);
		});
	});
});

it('the dark block used by the media query is the same as the explicit one', () => {
	expect(block('dark-media')).toEqual(schemes.dark);
	expect(raw('dark-media')).toEqual(raw('dark')); // scrim, shadow and color-scheme too
});
