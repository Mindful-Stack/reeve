import type { Epic, Initiative, RequiredDecision } from '#lib/api/types.ts';
import { blocking } from '#lib/decisions.ts';

/** The column for initiatives whose horizon is not configured any more, so a configuration change never hides work. */
export const OTHER = 'Other';

/** One entry per configured bucket, in configuration order (empty buckets included), plus `OTHER` only when some
 *  initiative's horizon is not configured. Each bucket is sorted by target month (initiatives without one last), then
 *  title. Bucket names come from the server's config; none is hardcoded here. */
export function bucketise(buckets: string[], initiatives: Initiative[]): Map<string, Initiative[]> {
	const out = new Map<string, Initiative[]>(buckets.map((b) => [b, []]));
	for (const i of initiatives) {
		const list = out.get(i.horizon);
		if (list) list.push(i);
		else {
			if (!out.has(OTHER)) out.set(OTHER, []);
			out.get(OTHER)!.push(i);
		}
	}
	for (const list of out.values()) list.sort(byTargetThenTitle);
	return out;
}

/** The planning page's layout over `bucketise`'s result: every configured horizon but the last is a column, then the
 *  synthetic `OTHER` column when present (a configured horizon named like it is just a horizon); the last configured
 *  horizon is `collapsed` (shown under the columns, not a drop target) unless `expanded`, when it is a column too, in
 *  configured order: after the other configured horizons, before the synthetic `OTHER`. With fewer than two configured
 *  horizons nothing is collapsed, so a single horizon is still a column. */
export function horizonLayout(
	configured: string[],
	buckets: Map<string, Initiative[]>,
	expanded = false
): { columns: string[]; collapsed: string | null } {
	const collapsed = configured.length > 1 ? configured[configured.length - 1] : null;
	const columns = collapsed === null || expanded ? [...configured] : configured.slice(0, -1);
	if (buckets.has(OTHER) && !configured.includes(OTHER)) columns.push(OTHER);
	return { columns, collapsed };
}

/** Every initiative in the planning page's order (`bucketise`, flattened): the order every initiative list and picker
 *  uses, so a picker matches what is on screen. */
export function planningOrder(buckets: string[], initiatives: Initiative[]): Initiative[] {
	return [...bucketise(buckets, initiatives).values()].flat();
}

function byTargetThenTitle(a: Initiative, b: Initiative): number {
	const am = a.target_month ?? null;
	const bm = b.target_month ?? null;
	if (am !== bm) {
		if (am === null) return 1;
		if (bm === null) return -1;
		return am < bm ? -1 : 1;
	}
	return a.title.localeCompare(b.title);
}

export type OpenDecisions = { count: number; oldest: string; open: RequiredDecision[] };

/** Open required decisions per item key (`on`): closed ones are excluded; `oldest` is the earliest `created_at`; `open`
 *  lists them oldest first, for the expanded card. Items without an open decision have no entry. */
export function openDecisionsByItem(decisions: RequiredDecision[]): Map<string, OpenDecisions> {
	const out = new Map<string, OpenDecisions>();
	for (const d of decisions) {
		if (d.closed) continue;
		const entry = out.get(d.on);
		if (entry) entry.open.push(d);
		else out.set(d.on, { count: 0, oldest: d.created_at, open: [d] });
	}
	for (const entry of out.values()) {
		entry.open.sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at));
		entry.count = entry.open.length;
		entry.oldest = entry.open[0].created_at;
	}
	return out;
}

/** Blocking decisions per item key: `openDecisionsByItem` over `blocking(decisions)`. */
export function blockingByItem(decisions: RequiredDecision[]): Map<string, OpenDecisions> {
	return openDecisionsByItem(blocking(decisions));
}

export type EpicGroup = { key: string | null; title: string; initiative: Initiative | null; epics: Epic[] };

/** The epics index: every initiative in the planning page's order (`bucketise`: configured horizons, then Other; target
 *  month, then title), each with its epics in the server's order, an empty list included; then a group per parent key
 *  not among `initiatives` (a list loaded mid-change), titled by the key; then "No initiative", only when some epic has
 *  none. */
export function groupEpics(buckets: string[], initiatives: Initiative[], epics: Epic[]): EpicGroup[] {
	const ordered = planningOrder(buckets, initiatives);
	const groups = new Map<string | null, EpicGroup>(
		ordered.map((i) => [i.key, { key: i.key, title: i.title, initiative: i, epics: [] }])
	);
	const none: EpicGroup = { key: null, title: 'No initiative', initiative: null, epics: [] };
	const unlisted: EpicGroup[] = [];
	for (const e of epics) {
		const k = e.initiative ?? null;
		if (k === null) {
			none.epics.push(e);
			continue;
		}
		let g = groups.get(k);
		if (!g) {
			g = { key: k, title: k, initiative: null, epics: [] };
			groups.set(k, g);
			unlisted.push(g);
		}
		g.epics.push(e);
	}
	return [...ordered.map((i) => groups.get(i.key)!), ...unlisted, ...(none.epics.length ? [none] : [])];
}
