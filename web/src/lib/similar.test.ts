import { describe, expect, it } from 'vitest';
import { similarQuery, withoutSelf } from './similar.ts';

const hit = (key: string) => ({ key, title: key, kind: 'story' as const, delivery: 'todo' as const });

describe('similarQuery', () => {
	it('is the trimmed title within 8..200 characters, counted as the server counts (code points), else null', () => {
		expect(similarQuery('  short ')).toBeNull();
		expect(similarQuery(' Blank PDF ')).toBe('Blank PDF');
		expect(similarQuery('x'.repeat(201))).toBeNull();
		expect(similarQuery('x'.repeat(7))).toBeNull();
		expect(similarQuery('x'.repeat(8))).toBe('x'.repeat(8));
		expect(similarQuery('x'.repeat(200))).toBe('x'.repeat(200));
		expect(similarQuery('😀'.repeat(150))).toBe('😀'.repeat(150)); // 300 UTF-16 units, 150 characters
	});
});

describe('withoutSelf', () => {
	it('drops the item itself and keeps the first five of the rest', () => {
		expect(withoutSelf(['GRA-5', 'A-1', 'A-2', 'A-3', 'A-4', 'A-5'].map(hit), 'GRA-5').map((h) => h.key)).toEqual(['A-1', 'A-2', 'A-3', 'A-4', 'A-5']);
		expect(withoutSelf(['A-1', 'A-2', 'A-3', 'A-4', 'A-5', 'A-6'].map(hit), 'GRA-5').map((h) => h.key)).toEqual(['A-1', 'A-2', 'A-3', 'A-4', 'A-5']);
	});
});
