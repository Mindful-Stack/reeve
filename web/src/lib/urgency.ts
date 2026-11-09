import type { ItemRef, UrgentMark } from '#lib/api/types.ts';
import { short } from '#lib/format.ts';

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
/** The chip's tooltip and the page badge line. `own` is the item's own mark when it is among the sources. */
export function urgentLine(
	own: UrgentMark | null | undefined,
	sources: ItemRef[],
	key: string,
	now: Date,
	titleOf: (r: ItemRef) => string | undefined
): string {
	const parts: string[] = [];
	if (own && sources[0]?.key === key) {
		parts.push(`marked by ${short(own.by)}, ${daysAgo(own.at, now)}`);
		if (own.reason) parts.push(`'${own.reason}'`);
		parts.push(expiry(own, now));
	}
	const others = sources.filter((s) => s.key !== key);
	if (others[0]) {
		const t = titleOf(others[0]);
		parts.push(
			`via ${others[0].item} ${others[0].key}${t ? ` ${t}` : ''}${others.length > 1 ? ` and ${others.length - 1} more` : ''}`
		);
	}
	return ['Urgent', ...parts].join(' · ');
}
