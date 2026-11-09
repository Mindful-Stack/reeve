import { describe, expect, it } from 'vitest';
import type { ItemRef, UrgentMark } from '#lib/api/types.ts';
import { daysAgo, expiry, itemRoute, ownMarkText, urgentLine, via } from './urgency.ts';

const now = new Date('2026-11-10T12:00:00Z');
const day = 86_400_000;
const iso = (offset: number) => new Date(now.getTime() + offset).toISOString();
const mark = (over: Partial<UrgentMark> = {}): UrgentMark => ({
	at: iso(-2 * day),
	by: 'dana@example.com',
	reason: null,
	expires_at: iso(5 * day),
	active: true,
	done: false,
	...over
});
const ref = (key: string, item: ItemRef['item'] = 'initiative'): ItemRef => ({ key, item });

describe('daysAgo', () => {
	it('floors whole days and says today under a day', () => {
		expect(daysAgo(iso(-2.9 * day), now)).toBe('2 d ago');
		expect(daysAgo(iso(-3600_000), now)).toBe('today');
	});
	it('never goes negative', () => {
		expect(daysAgo(iso(3 * day), now)).toBe('today');
	});
});

describe('expiry', () => {
	it('counts the days left of an active mark', () => {
		expect(expiry(mark(), now)).toBe('expires in 5 d');
	});
	it('says today within the last day', () => {
		expect(expiry(mark({ expires_at: iso(3600_000) }), now)).toBe('expires today');
	});
	it('says how long ago an expired mark ran out', () => {
		expect(expiry(mark({ active: false, expires_at: iso(-3 * day) }), now)).toBe('expired 3 d ago');
	});
});

describe('urgentLine', () => {
	const titles = (r: ItemRef) => (r.key === 'ABC-311' ? 'Payments outage' : undefined);
	it('describes an own mark', () => {
		expect(urgentLine(mark({ reason: 'Customer escalation' }), [ref('ABC-1')], 'ABC-1', now, titles)).toBe(
			"Urgent · marked by dana, 2 d ago · 'Customer escalation' · expires in 5 d"
		);
	});
	it('describes a derived urgency by its source', () => {
		expect(urgentLine(null, [ref('ABC-311', 'story')], 'ABC-1', now, titles)).toBe('Urgent · via story ABC-311 Payments outage');
	});
	it('describes own plus others', () => {
		expect(urgentLine(mark(), [ref('ABC-1'), ref('ABC-311', 'story'), ref('ABC-9', 'epic')], 'ABC-1', now, titles)).toBe(
			'Urgent · marked by dana, 2 d ago · expires in 5 d · via story ABC-311 Payments outage and 1 more'
		);
	});
	it('ignores an expired own mark that is not among the sources', () => {
		expect(urgentLine(mark({ active: false, expires_at: iso(-day) }), [ref('ABC-311', 'story')], 'ABC-1', now, titles)).toBe(
			'Urgent · via story ABC-311 Payments outage'
		);
	});
});

describe('ownMarkText, via, itemRoute', () => {
	it('says who, when, why and the expiry', () => {
		expect(ownMarkText(mark({ reason: 'Burning' }), now)).toBe("marked by dana, 2 d ago · 'Burning' · expires in 5 d");
	});
	it('finds the first other source and counts the rest', () => {
		const t = (r: ItemRef) => (r.key === 'ABC-9' ? 'Nine' : undefined);
		expect(via([ref('ABC-1'), ref('ABC-9', 'story'), ref('ABC-2', 'epic')], 'ABC-1', t)).toEqual({ ref: ref('ABC-9', 'story'), title: 'Nine', more: 1 });
		expect(via([ref('ABC-1')], 'ABC-1', t)).toBeNull();
	});
	it('routes a story to its drawer and the others to their pages', () => {
		const url = { href: 'http://x/epics/ABC-5' };
		expect(itemRoute(ref('ABC-9', 'story'), url)).toBe('/epics/ABC-5?story=ABC-9');
		expect(itemRoute(ref('ABC-5', 'epic'), url)).toBe('/epics/ABC-5');
		expect(itemRoute(ref('ABC-1'), url)).toBe('/initiatives/ABC-1');
	});
	it('finds the own mark anywhere among the sources', () => {
		expect(urgentLine(mark(), [ref('ABC-9', 'story'), ref('ABC-1')], 'ABC-1', now, () => undefined)).toContain('marked by dana');
	});
});
