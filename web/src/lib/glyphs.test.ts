import { describe, expect, it } from 'vitest';
import { glyph, levelOfItem, levelOfKind } from './glyphs.ts';

describe('glyphs', () => {
	it('gives each level the CLI glyph', () => {
		expect(['initiative', 'epic', 'story', 'bug', 'chore'].map((l) => glyph(l as never))).toEqual(['◆', '■', '●', '▲', '○']);
		expect(levelOfKind('bug')).toBe('bug');
		expect(levelOfItem('epic')).toBe('epic');
	});
});
