/** The story drawer's link: the current URL (path and query) with `?story=KEY`, so the drawer opens over this view.
 *  Any decision drawer closes: the two are mutually exclusive. */
export function storyHref(url: { readonly href: string }, key: string): string {
	const next = new URL(url.href);
	next.searchParams.delete('decision');
	next.searchParams.set('story', key);
	return next.pathname + next.search;
}

/** The decision drawer's link: the current URL with `?decision=ID` (any story drawer closes). */
export function decisionHref(url: { readonly href: string }, id: number): string {
	const next = new URL(url.href);
	next.searchParams.delete('story');
	next.searchParams.set('decision', String(id));
	return next.pathname + next.search;
}
