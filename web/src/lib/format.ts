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

/** "2026-11-20" → "20 Nov 2026": `dayLabel` with the year, for words that stay on a done item long after the day.
 *  Anything else is returned unchanged. */
export function dateLabel(iso: string): string {
	const d = dayLabel(iso);
	return d === iso ? iso : `${d} ${iso.slice(0, 4)}`;
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

const payloadField = (o: unknown, name: string): unknown => (o && typeof o === 'object' && name in o ? (o as Record<string, unknown>)[name] : undefined);
const nonEmpty = (v: unknown): string | null => (typeof v === 'string' && v !== '' ? v : null);

/** A `promised` entry's words, chosen by comparing `on` and `to` with `previous` (null for a new promise). `key` names
 *  the item, for a note that is not already about it. */
function promisedWords(payload: unknown, key: string | undefined): string {
	const on = nonEmpty(payloadField(payload, 'on'));
	if (on == null) return key == null ? 'promised' : `promised ${key}`;
	const to = nonEmpty(payloadField(payload, 'to'));
	const prev = payloadField(payload, 'previous');
	const promise = key == null ? 'the promise' : `the promise on ${key}`;
	const recipient = key == null ? "the promise's recipient" : `the recipient of the promise on ${key}`;
	const fresh = `promised ${key == null ? '' : `${key} `}by ${dateLabel(on)}${to == null ? '' : ` to ${to}`}`;
	if (prev == null || typeof prev !== 'object') return fresh;
	const prevOn = nonEmpty(payloadField(prev, 'on'));
	const prevTo = nonEmpty(payloadField(prev, 'to'));
	if (on !== prevOn) {
		const change = `changed ${promise} to ${dateLabel(on)}`;
		if (to === prevTo) return change;
		return to == null ? `${change} and removed its recipient` : `${change} and its recipient to ${to}`;
	}
	if (to === prevTo) return fresh;
	return to == null ? `removed ${recipient}` : `changed ${recipient} to ${to}`;
}

/** An activity entry's action in its own words: an attachment's names its file, a promise's its date and recipient (from
 *  the payload); null for every other action, which reads as `plainAction`. The one place an action is worded. `key`
 *  names the item in a promise's words, where the note is not already about one item (`freshNote`). */
export function actionLabel(action: string, payload: unknown, key?: string): string | null {
	if (action === 'attachment_added' || action === 'attachment_removed') {
		const name = nonEmpty(payloadField(payload, 'filename'));
		return `${action === 'attachment_added' ? 'added' : 'deleted'} attachment${name == null ? '' : ` ${name}`}`;
	}
	if (action === 'promised') return promisedWords(payload, key);
	if (action === 'promise_cleared') return key == null ? 'cleared the promise' : `cleared the promise on ${key}`;
	return null;
}

/** An action `actionLabel` has no words for, as people read it: the action with spaces ("goal linked"). */
export function plainAction(action: string): string {
	return action.replaceAll('_', ' ');
}

/** " via claude.ai" for an entry of a write made through an MCP client (kb/ADR-0029 R9: the server records the client
 *  in the payload), else "". The one place the client is worded; it is appended to an entry's words. */
export function viaClient(payload: unknown): string {
	const client = nonEmpty(payloadField(payload, 'client'));
	return client == null ? '' : ` via ${client}`;
}

/** An activity entry's action as the activity views show it: `actionLabel`'s words, or the plain action, and, for a
 *  write made through an MCP client, "via" and the client. */
export function entryAction(action: string, payload: unknown): string {
	return `${actionLabel(action, payload) ?? plainAction(action)}${viaClient(payload)}`;
}

/** `n` and the noun that agrees with it: `plural(1, 'day', 'days')` is "1 day". */
export function plural(n: number, one: string, many: string): string {
	return `${n} ${n === 1 ? one : many}`;
}
