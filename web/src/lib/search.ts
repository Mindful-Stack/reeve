import type { SearchHit, Story } from '#lib/api/types.ts';
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

/** Text that names a key (`12`, `#12`, `GRA-12`, any case) or a Jira-style reference (`SCRUM-40`), as the server reads
 *  one: the header box looks it up before filtering the backlog by it. A phrase ("12 apples") is never key-shaped. */
export function isKeyShaped(text: string): boolean {
	return /^(#?\d+|[a-z][a-z0-9]*-\d+)$/i.test(text.trim());
}

/** Why a story the backlog's text matched is listed when its title does not show it (the title is highlighted when it
 *  does): read from the row's own fields, in the server's order of fields; the key is what is left. */
export function matchedBy(s: Story, query: string | null): string | null {
	const q = query?.trim().toLowerCase();
	if (!q || s.title.toLowerCase().includes(q)) return null;
	if (s.description.toLowerCase().includes(q)) return 'matched description';
	if (s.external_ref?.toLowerCase().includes(q)) return `matched ${s.external_ref}`;
	return 'matched key';
}
