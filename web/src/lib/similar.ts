// web/src/lib/similar.ts: the rules both places that ask `GET /stories/similar` share (the Report dialog and triage): the
// text the server compares, how many hits are shown, and how a hit reads.
import type { SimilarStory } from '#lib/api/types.ts';

/** The server compares 8 to 200 characters of a title, counted in code points. */
export const QUERY_MIN = 8;
export const QUERY_MAX = 200;
/** Hits shown for one item. */
export const SIMILAR_SHOWN = 5;

export const points = (s: string): number => [...s].length;

/** The text `GET /stories/similar` accepts (the trimmed title within the bounds), or null when the title is outside them. */
export function similarQuery(title: string): string | null {
	const t = title.trim();
	const n = points(t);
	return n >= QUERY_MIN && n <= QUERY_MAX ? t : null;
}

/** The server's similar list includes the item itself: drop it and keep the first few. */
export function withoutSelf(hits: SimilarStory[], key: string): SimilarStory[] {
	return hits.filter((h) => h.key !== key).slice(0, SIMILAR_SHOWN);
}
