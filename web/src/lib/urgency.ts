import type { ItemRef, UrgentMark } from '#lib/api/types.ts';
import { short } from '#lib/format.ts';
import { storyHref } from '#lib/drawer.ts';

const DAY = 86_400_000;
/** "2 d ago", "today": whole days between two instants, never negative. */
export function daysAgo(at: string, now: Date): string {
	const d = Math.max(0, Math.floor((now.getTime() - new Date(at).getTime()) / DAY));
	return d === 0 ? 'today' : `${d} d ago`;
}
/** "expires in 5 d", "expires today", "expired 3 d ago" — from the server's expires_at. */
export function expiry(mark: UrgentMark, now: Date): string {
	const ms = new Date(mark.expires_at).getTime() - now.getTime();
	if (mark.active) {
		const d = Math.ceil(ms / DAY);
		return d <= 1 ? 'expires today' : `expires in ${d} d`;
	}
	return `expired ${daysAgo(mark.expires_at, now)}`;
}
/** "marked by dana, 2 d ago · 'reason' · expires in 5 d": the own-mark part of a line. */
export function ownMarkText(mark: UrgentMark, now: Date): string {
	const parts = [`marked by ${short(mark.by)}, ${daysAgo(mark.at, now)}`];
	if (mark.reason) parts.push(`'${mark.reason}'`);
	parts.push(expiry(mark, now));
	return parts.join(' · ');
}
/** The first source that is not the item itself, its title when known, and how many more there are. */
export function via(
	sources: ItemRef[],
	key: string,
	titleOf: (r: ItemRef) => string | undefined
): { ref: ItemRef; title: string | undefined; more: number } | null {
	const others = sources.filter((s) => s.key !== key);
	return others[0] ? { ref: others[0], title: titleOf(others[0]), more: others.length - 1 } : null;
}
/** Where an item's page is; a story opens the drawer over `url`'s page. */
export function itemRoute(r: ItemRef, url: { readonly href: string }): string {
	if (r.item === 'story') return storyHref(url, r.key);
	return `/${r.item === 'initiative' ? 'initiatives' : 'epics'}/${encodeURIComponent(r.key)}`;
}
/** The chip's tooltip and the page badge line. `own` is the item's own mark when it is among the sources. */
export function urgentLine(
	own: UrgentMark | null | undefined,
	sources: ItemRef[],
	key: string,
	now: Date,
	titleOf: (r: ItemRef) => string | undefined
): string {
	const parts: string[] = [];
	if (own && sources.some((s) => s.key === key)) parts.push(ownMarkText(own, now));
	const v = via(sources, key, titleOf);
	if (v) parts.push(`via ${v.ref.item} ${v.ref.key}${v.title ? ` ${v.title}` : ''}${v.more > 0 ? ` and ${v.more} more` : ''}`);
	return ['Urgent', ...parts].join(' · ');
}
