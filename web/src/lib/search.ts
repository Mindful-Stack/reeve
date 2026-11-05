import type { SearchHit } from '#lib/api/types.ts';
import { levelOfItem, levelOfKind, type Level } from '#lib/glyphs.ts';

/** Where a hit opens: an initiative's or an epic's page, a story in the board's drawer. */
export function hitHref(h: SearchHit): string {
	const k = encodeURIComponent(h.key);
	if (h.item === 'initiative') return `/initiatives/${k}`;
	if (h.item === 'epic') return `/epics/${k}`;
	return `/board?story=${k}`;
}

/** A story's level is its kind; the others are their item type. */
export function hitLevel(h: SearchHit): Level {
	return h.item === 'story' && h.kind ? levelOfKind(h.kind) : levelOfItem(h.item);
}

export type Piece = { text: string; match: boolean };

/** Splits a title around every case-insensitive occurrence of the query; the pieces rejoin to the title. Matching runs
 *  on the title itself (an escaped `iu` RegExp), so a character whose lower case is longer ('İ') shifts no offset. */
export function highlight(title: string, query: string): Piece[] {
	const q = query.trim();
	if (!q) return [{ text: title, match: false }];
	const pieces: Piece[] = [];
	const needle = new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), 'giu');
	let at = 0;
	for (const m of title.matchAll(needle)) {
		if (m.index > at) pieces.push({ text: title.slice(at, m.index), match: false });
		pieces.push({ text: m[0], match: true });
		at = m.index + m[0].length;
	}
	if (at < title.length) pieces.push({ text: title.slice(at), match: false });
	return pieces.length ? pieces : [{ text: title, match: false }];
}
