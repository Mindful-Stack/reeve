const MINUTE = 60_000;
const HOUR = 3_600_000;
const DAY = 86_400_000;
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** Time since `iso`, coarsely: minutes ("0m".."59m") and hours ("1h".."23h") under a day, then "1d".."13d", weeks up to
 *  "8w", then months ("5mo"); "?" for an unparseable date. A future date counts as now. */
export function age(iso: string, now: number = Date.now()): string {
	const t = Date.parse(iso);
	if (Number.isNaN(t)) return '?';
	const ms = Math.max(0, now - t);
	if (ms < HOUR) return `${Math.floor(ms / MINUTE)}m`;
	if (ms < DAY) return `${Math.floor(ms / HOUR)}h`;
	const days = Math.floor(ms / DAY);
	if (days < 14) return `${days}d`;
	if (days < 60) return `${Math.floor(days / 7)}w`;
	return `${Math.floor(days / 30)}mo`;
}

/** "2026-11" → "Nov 2026". Anything else is returned unchanged. */
export function month(ym: string): string {
	const m = /^(\d{4})-(\d{2})$/.exec(ym);
	const i = m ? Number(m[2]) - 1 : -1;
	return m && i >= 0 && i < 12 ? `${MONTHS[i]} ${m[1]}` : ym;
}

/** "2026-11-20" → "20 Nov". Built from the string's parts, never a `Date`, so no time zone can shift the day.
 *  Anything else is returned unchanged. */
export function dayLabel(iso: string): string {
	const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
	const i = m ? Number(m[2]) - 1 : -1;
	const d = m ? Number(m[3]) : 0;
	return m && i >= 0 && i < 12 && d >= 1 && d <= 31 ? `${d} ${MONTHS[i]}` : iso;
}

/** The local part of an email address. */
export function short(email: string): string {
	const at = email.indexOf('@');
	return at > 0 ? email.slice(0, at) : email;
}

/** The text to show for anything thrown: an Error's message (an `ApiError` carries the server's), or the value itself. */
export function errorMessage(e: unknown): string {
	return e instanceof Error ? e.message : String(e);
}

/** An activity entry's action as people read it: an attachment's names its file (from the payload); any other is its
 *  action with spaces. */
export function actionLabel(action: string, payload: unknown): string {
	if (action === 'attachment_added' || action === 'attachment_removed') {
		const name =
			payload && typeof payload === 'object' && 'filename' in payload && typeof payload.filename === 'string' ? ` ${payload.filename}` : '';
		return `${action === 'attachment_added' ? 'added' : 'deleted'} attachment${name}`;
	}
	return action.replaceAll('_', ' ');
}

/** `n` and the noun that agrees with it: `plural(1, 'day', 'days')` is "1 day". */
export function plural(n: number, one: string, many: string): string {
	return `${n} ${n === 1 ? one : many}`;
}
