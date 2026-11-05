import { describe, expect, it } from 'vitest';
import { contrast, hexToRgb } from './color.ts';

describe('color', () => {
	it('parses #rrggbb in either case and refuses anything else', () => {
		expect(hexToRgb('#FaF4eD')).toEqual([250, 244, 237]);
		for (const bad of ['red', '#abc', '#aabbccdd', 'url(x)', '#a1b2c3;}']) expect(() => hexToRgb(bad)).toThrow();
	});
	it('computes WCAG 2 contrast', () => {
		expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 5);
		expect(contrast('#ffffff', '#000000')).toBeCloseTo(21, 5); // order does not matter
		expect(contrast('#777777', '#ffffff')).toBeCloseTo(4.48, 2); // the classic just-failing grey
	});
});
