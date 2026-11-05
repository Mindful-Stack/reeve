/** The story drawer's link: the current URL (path and query) with `?story=KEY`, so the drawer opens over this view. */
export function storyHref(url: { readonly href: string }, key: string): string {
	const next = new URL(url.href);
	next.searchParams.set('story', key);
	return next.pathname + next.search;
}
