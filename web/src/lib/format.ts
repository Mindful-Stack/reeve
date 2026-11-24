import { LINK_TYPES, linkTypeText } from '#lib/relations.ts';

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

/** Emails compare case-insensitively and without surrounding whitespace. */
export function sameEmail(a: string, b: string): boolean {
	return a.trim().toLowerCase() === b.trim().toLowerCase();
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

/** An item link entry's words: both items and the name between them, as stored. The same entry is logged on both
 *  items (kb/ADR-0031 R7), so the words name both rather than saying "this". The name is worded by `linkTypeText`,
 *  the one function that words a name; a payload whose `type` is not a name this client knows gets the words
 *  without the link. */
function itemLinkWords(action: string, payload: unknown): string {
	const verb = action === 'item_link_added' ? 'added' : 'removed';
	const stored = payloadField(payload, 'type');
	const type = LINK_TYPES.find((t) => t === stored);
	const from = nonEmpty(payloadField(payload, 'from'));
	const to = nonEmpty(payloadField(payload, 'to'));
	return type === undefined || from == null || to == null ? `${verb} item link` : `${verb} item link: ${from} ${linkTypeText(type)} ${to}`;
}

/** An `edited` entry's words when the edit moved an epic to another initiative or out of one: the payload names
 *  every changed field (`fields`, which then holds "initiative") and the key it left and joined (`from.initiative`,
 *  `to.initiative`, null for none). Null for every other edit, a retitle or a new brief included, which reads as
 *  "edited". */
function epicMoveWords(payload: unknown): string | null {
	const fields = payloadField(payload, 'fields');
	if (!Array.isArray(fields) || !fields.includes('initiative')) return null;
	const from = nonEmpty(payloadField(payloadField(payload, 'from'), 'initiative'));
	const to = nonEmpty(payloadField(payloadField(payload, 'to'), 'initiative'));
	if (from != null && to != null) return `edited and moved from initiative ${from} to ${to}`;
	if (to != null) return `edited and moved to initiative ${to}`;
	return from == null ? null : `edited and taken out of initiative ${from}`;
}

/** An activity entry's action in its own words: an attachment's names its file, a promise's its date and recipient, an
 *  item link's its two items and its name, a web link's its title and, once removed, its URL, an epic's edit that moved
 *  it its two initiatives (from the payload); null
 *  for every other action, which reads as `plainAction`. The one place an action is worded. `key` names the item in a
 *  promise's words, where the note is not already about one item (`freshNote`). */
export function actionLabel(action: string, payload: unknown, key?: string): string | null {
	if (action === 'attachment_added' || action === 'attachment_removed') {
		const name = nonEmpty(payloadField(payload, 'filename'));
		return `${action === 'attachment_added' ? 'added' : 'deleted'} attachment${name == null ? '' : ` ${name}`}`;
	}
	if (action === 'promised') return promisedWords(payload, key);
	if (action === 'promise_cleared') return key == null ? 'cleared the promise' : `cleared the promise on ${key}`;
	if (action === 'item_link_added' || action === 'item_link_removed') return itemLinkWords(action, payload);
	if (action === 'web_link_added' || action === 'web_link_removed') {
		const title = nonEmpty(payloadField(payload, 'title'));
		const named = title == null ? '' : ` ${title}`;
		if (action === 'web_link_added') return `added web link${named}`;
		// A removed web link is listed nowhere any more, so its entry also says where it pointed.
		const url = nonEmpty(payloadField(payload, 'url'));
		return `removed web link${named}${url == null ? '' : ` (${url})`}`;
	}
	if (action === 'edited') return epicMoveWords(payload);
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
