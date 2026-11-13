/** The story drawer's link: the current URL (path, query and hash) with `?story=KEY`, so the drawer opens over this view.
 *  Any decision drawer closes: the two are mutually exclusive. The hash stays, so a lane link (`/decisions#next-sprint`)
 *  keeps its lane. Anchors to these links carry `data-sveltekit-reset="false"`: the view behind a drawer must not move. */
export function storyHref(url: { readonly href: string }, key: string): string {
	const next = new URL(url.href);
	next.searchParams.delete('decision');
	next.searchParams.set('story', key);
	return next.pathname + next.search + next.hash;
}

/** The decision drawer's link: the current URL with `?decision=ID` (any story drawer closes). */
export function decisionHref(url: { readonly href: string }, id: number): string {
	const next = new URL(url.href);
	next.searchParams.delete('story');
	next.searchParams.set('decision', String(id));
	return next.pathname + next.search + next.hash;
}

/** The decision id a URL opens, or null: anything but a positive whole number opens nothing. */
export function decisionId(url: { readonly searchParams: Pick<URLSearchParams, 'get'> }): number | null {
	const raw = url.searchParams.get('decision');
	return raw !== null && /^[1-9]\d*$/.test(raw) ? Number(raw) : null;
}

/** Whether the URL opens a drawer, by the rule the layout opens one with: a story drawer for a non-empty key, a decision
 *  drawer for a decision id. A page that must stand back while a drawer is over it asks this, not for the parameter. */
export function hasDrawer(url: { readonly searchParams: Pick<URLSearchParams, 'get'> }): boolean {
	return !!url.searchParams.get('story') || decisionId(url) !== null;
}
