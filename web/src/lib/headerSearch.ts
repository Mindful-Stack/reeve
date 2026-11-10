// The header box: the app's one text box (kb/ADR-0021). A key or Jira reference jumps to the item it names exactly; any
// other text filters the backlog by it, which links to /search for epics, initiatives and done work.
import type { SearchHit } from '#lib/api/types.ts';
import { withoutText } from '#lib/backlog.ts';
import { storyHref } from '#lib/drawer.ts';
import { hitHref, isKeyShaped } from '#lib/search.ts';

/** Where a submission goes: `story` opens the drawer over the current page, `item` an epic's or initiative's page,
 *  `filter` the backlog filtered by the text (on /backlog, the same page with `q` set). */
export type HeaderTarget = { href: string; kind: 'story' | 'item' | 'filter' };

/** The target for `text` submitted on `url`. Key-shaped text is looked up first (`lookup` is `GET /search`); an exact
 *  first hit jumps, anything else (no exact hit, a failed lookup) falls through to the backlog. Empty text on a backlog
 *  with text drops it (and relevance, as the chip does); elsewhere it goes nowhere (null). */
export async function headerTarget(text: string, url: URL, lookup: (q: string) => Promise<SearchHit[]>): Promise<HeaderTarget | null> {
	const q = text.trim();
	if (!q) {
		if (url.pathname !== '/backlog' || !url.searchParams.has('q')) return null;
		const next = withoutText(url);
		return { href: next.pathname + next.search, kind: 'filter' };
	}
	if (isKeyShaped(q)) {
		try {
			const first = (await lookup(q))[0];
			if (first?.exact) return first.item === 'story' ? { href: storyHref(url, first.key), kind: 'story' } : { href: hitHref(first), kind: 'item' };
		} catch {
			// The lookup failed: the text still filters the backlog.
		}
	}
	if (url.pathname !== '/backlog') return { href: `/backlog?${new URLSearchParams({ q, state: 'open', view: 'relevance' })}`, kind: 'filter' };
	const next = new URL(url.href);
	const p = next.searchParams;
	// The first text on a backlog with no chosen view sorts by relevance; a chosen view (or the legacy group=rank) stays.
	const relevance = !p.get('q')?.trim() && !p.has('view') && !p.has('group');
	p.set('q', q);
	if (relevance) p.set('view', 'relevance');
	return { href: next.pathname + next.search, kind: 'filter' };
}

/** The layout's submit: every submission (key or text) supersedes the ones before it, so a lookup that answers late
 *  never navigates; nor does one whose page moved on meanwhile (`current()`, the page's href, differs from `url`'s), since
 *  its target was built from the page as it was. `onbusy` is true while the latest submission is pending. */
export function headerSearch(
	lookup: (q: string) => Promise<SearchHit[]>,
	navigate: (to: HeaderTarget) => Promise<unknown>,
	onbusy: (busy: boolean) => void,
	current: () => string
): (text: string, url: URL) => Promise<void> {
	let latest = 0;
	return async (text, url) => {
		const seq = ++latest;
		onbusy(true);
		try {
			const to = await headerTarget(text, url, lookup);
			if (seq === latest && to && current() === url.href) await navigate(to);
		} finally {
			if (seq === latest) onbusy(false);
		}
	};
}
