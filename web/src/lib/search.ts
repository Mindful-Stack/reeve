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
/** The query as a case-insensitive RegExp matching it literally; `highlight` and `matchedBy` share it, so a tag never
 *  disagrees with a highlight. */
function needle(q: string, flags: 'iu' | 'giu'): RegExp {
	return new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), flags);
}

export function highlight(title: string, query: string): Piece[] {
	const q = query.trim();
	if (!q) return [{ text: title, match: false }];
	const pieces: Piece[] = [];
	const found = needle(q, 'giu');
	let at = 0;
	for (const m of title.matchAll(found)) {
		if (m.index > at) pieces.push({ text: title.slice(at, m.index), match: false });
		pieces.push({ text: m[0], match: true });
		at = m.index + m[0].length;
	}
	if (at < title.length) pieces.push({ text: title.slice(at), match: false });
	return pieces.length ? pieces : [{ text: title, match: false }];
}

/** Text that names a key (`12`, `#12`, `GRA-12`, any case) or a Jira-style reference (`SCRUM-40`): the header box looks
 *  it up before filtering the backlog by it. A phrase ("12 apples") is never key-shaped. Every text the server's
 *  `key_named_by` (here `keyNamedBy`) reads as a key is key-shaped; the shape is looser (a leading zero, a reference
 *  under another prefix), because `GET /search` decides whether it names anything. */
export function isKeyShaped(text: string): boolean {
	return /^(#?\d+|[a-z][a-z0-9]*-\d+)$/i.test(text.trim());
}

/** The key `text` names, as the server's `key_named_by` reads it (keep the two in step): a bare number or `#n` under
 *  `prefix`, or a key in any case. Null for anything else. */
export function keyNamedBy(text: string, prefix: string | null | undefined): string | null {
	const q = text.trim();
	const n = q.startsWith('#') ? q.slice(1) : q;
	if (/^\d+$/.test(n)) return prefix && /^[1-9]/.test(n) ? `${prefix}-${n}` : null;
	const upper = q.toUpperCase();
	return /^[A-Z][A-Z0-9]*-[1-9]\d*$/.test(upper) ? upper : null;
}

/** Why a story the backlog's text matched is listed when its title does not show it (the title is highlighted when it
 *  does): read from the row's own fields with `highlight`'s RegExp, in the server's order of fields. "matched key" only
 *  when the text names this row's key (`keyNamedBy`, the server's `key_named_by`); a match the row's fields do not
 *  explain gets no tag rather than a guess. */
export function matchedBy(s: Story, query: string | null, prefix: string | null | undefined): string | null {
	const q = query?.trim();
	if (!q) return null;
	const found = needle(q, 'iu');
	if (found.test(s.title)) return null;
	if (found.test(s.description)) return 'matched description';
	if (s.external_ref && found.test(s.external_ref)) return `matched ${s.external_ref}`;
	return keyNamedBy(q, prefix) === s.key ? 'matched key' : null;
}
