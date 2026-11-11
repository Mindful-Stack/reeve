// web/src/lib/nav.ts: the main navigation destinations, and the context a Report started from a page captures. The
// header button and the drawer's button both build it here, so one page gives one context.
import type { Origin } from '#lib/report.svelte.ts';

export const links = [
	{ href: '/', label: 'Home' },
	{ href: '/planning', label: 'Planning' },
	{ href: '/backlog', label: 'Backlog' },
	{ href: '/board', label: 'Board' },
	{ href: '/sprint', label: 'Sprint' },
	{ href: '/decisions', label: 'Decisions' }
];

type Where = { pathname: string; search: string };

export const isCurrent = (url: Where, href: string): boolean => (href === '/' ? url.pathname === '/' : url.pathname.startsWith(href));

/** A malformed %-sequence leaves the raw segment rather than throwing. */
function decode(s: string): string {
	try {
		return decodeURIComponent(s);
	} catch {
		return s;
	}
}

/** Where a report starts: the path with its query, the section label (the epic's key on an epic page), and the epic
 *  key a chip may name (epic pages only; the drawer never asks for it). */
export function reportContext(url: Where): Origin & { epicKey: string | null } {
	const epicKey = url.pathname.match(/^\/epics\/([^/]+)$/)?.[1];
	const key = epicKey ? decode(epicKey) : null;
	return {
		path: url.pathname + url.search,
		label: key ?? links.find((l) => isCurrent(url, l.href))?.label ?? url.pathname,
		epicKey: key
	};
}
