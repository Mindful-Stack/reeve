import type { ActivityEntry } from '#lib/api/types.ts';
import { createSubscriber } from 'svelte/reactivity';
import { short } from '#lib/format.ts';

/** How long a change by someone else stays highlighted on its card or row. */
export const RECENT_MS = 10_000;
/** The most arrivals the window keeps; the oldest go first. */
export const RECENT_MAX = 200;
/** The shortest wait before the expiry timer runs again, so an arrival already at its end of window never spins it. */
const EXPIRY_RETRY_MS = 16;
export type Arrival = { entry: ActivityEntry; arrived: number };

/** The tab's view of `GET /api/v1/events`. A mutable rune object: fields are assigned, the binding never is.
 *  `version` goes up on every activity entry and on every (re)connect, since an outage may have hidden changes. */
export const live = $state<{
	connected: boolean;
	lastId: number;
	version: number;
	latest: ActivityEntry | null;
	/** Entries that arrived in the last RECENT_MS, oldest first, compacted to the latest per entity key and actor and
	 *  capped at RECENT_MAX (the oldest go first). Display only: refetching still follows onLive. */
	recent: Arrival[];
}>({ connected: false, lastId: 0, version: 0, latest: null, recent: [] });

type StreamHooks = { entry?: (e: ActivityEntry) => void; reopened?: () => void };
const hooks = new Set<StreamHooks>();
let everOpened = false;

/** Runs `entry` for every activity entry this tab receives, in order, and `reopened` on every open of the stream after
 *  the first (an outage may have hidden entries). For state that must see each entry, which `onLive`'s debounced
 *  refetch does not promise. Returns the unsubscribe. */
export function onStream(h: StreamHooks): () => void {
	hooks.add(h);
	return () => hooks.delete(h);
}

/** Test-only: forget the first open and every hook. */
export function resetStreamForTests(): void {
	hooks.clear();
	everOpened = false;
}

let expiry: ReturnType<typeof setTimeout> | undefined;

/** One timer, set for the oldest arrival's end of window: prunes, then schedules again while the window is non-empty. */
function schedule() {
	clearTimeout(expiry);
	expiry = undefined;
	if (live.recent.length === 0) return;
	const wait = Math.max(EXPIRY_RETRY_MS, live.recent[0].arrived + RECENT_MS - Date.now());
	expiry = setTimeout(() => {
		live.recent = live.recent.filter((r) => Date.now() - r.arrived < RECENT_MS);
		schedule();
	}, wait);
}

function clearRecent() {
	clearTimeout(expiry);
	expiry = undefined;
	live.recent = [];
}

/** "erik moved this · just now" while someone other than `me` changed `key` within the window, else null. */
export function freshNote(recent: Arrival[], key: string, me: string | undefined, now: number = Date.now()): string | null {
	for (let i = recent.length - 1; i >= 0; i--) {
		const { entry, arrived } = recent[i];
		if (now - arrived >= RECENT_MS) return null;
		if (entry.entity_key !== key || entry.actor === me) continue;
		return `${short(entry.actor)} ${entry.action.replaceAll('_', ' ')} this · just now`;
	}
	return null;
}

/** Opens one EventSource for this tab (cookie auth; a GET needs no CSRF header). The browser reconnects by itself and
 *  resends `Last-Event-ID`, so the stream resumes where it stopped. `onclosed` runs when the browser gives up instead
 *  (`readyState === CLOSED` after an error): the server refused the reconnect, typically because the session died.
 *  Returns the disposer. */
export function connect(onclosed?: () => void): () => void {
	const es = new EventSource('/api/v1/events');
	es.addEventListener('cursor', (e) => {
		live.lastId = Number((e as MessageEvent).data);
	});
	es.addEventListener('activity', (e) => {
		const m = e as MessageEvent<string>;
		live.lastId = Number(m.lastEventId);
		const entry = JSON.parse(m.data) as ActivityEntry;
		live.latest = entry;
		live.version++;
		for (const h of hooks) h.entry?.(entry);
		const arrived = Date.now();
		live.recent = [
			...live.recent.filter((r) => arrived - r.arrived < RECENT_MS && !(r.entry.entity_key === entry.entity_key && r.entry.actor === entry.actor)),
			{ entry, arrived }
		].slice(-RECENT_MAX);
		schedule();
	});
	es.onopen = () => {
		live.connected = true;
		live.version++;
		const again = everOpened;
		everOpened = true;
		if (again) for (const h of hooks) h.reopened?.();
	};
	es.onerror = () => {
		live.connected = false;
		if (es.readyState === EventSource.CLOSED) {
			clearRecent();
			onclosed?.();
		}
	};
	return () => {
		es.close();
		live.connected = false;
		clearRecent();
	};
}

/** Calls `refetch` after a change to `live.version` seen since the component mounted: 500 ms after the last change,
 *  and at most `maxWait` after the first change of a burst, so a steady stream refetches every few seconds rather than
 *  never. A hidden tab refetches nothing: a change (or a refetch already scheduled when the tab is hidden) marks the
 *  view stale, and the tab refetches once when it is shown again. No filtering by entity: a story change moves
 *  initiative progress and a discovery mark changes every descendant's readiness, so the dependency graph is the whole
 *  tree and a full refetch is the honest answer. Call during component initialisation. */
export function onLive(refetch: () => unknown, delay = 500, maxWait = 5000): void {
	let seen = live.version;
	let stale = false;
	let trailing: ReturnType<typeof setTimeout> | undefined;
	let capped: ReturnType<typeof setTimeout> | undefined;
	const cancel = () => {
		clearTimeout(trailing);
		clearTimeout(capped);
		trailing = capped = undefined;
	};
	const fire = () => {
		cancel();
		stale = false;
		refetch();
	};
	const hidden = () => document.visibilityState === 'hidden';
	const onVisibility = () => {
		if (hidden()) {
			if (trailing !== undefined) {
				cancel();
				stale = true;
			}
		} else if (stale) {
			fire();
		}
	};
	$effect(() => {
		const v = live.version;
		if (v === seen) return;
		seen = v;
		if (hidden()) {
			stale = true;
			return;
		}
		clearTimeout(trailing);
		trailing = setTimeout(fire, delay);
		capped ??= setTimeout(fire, maxWait);
	});
	$effect(() => {
		document.addEventListener('visibilitychange', onVisibility);
		return () => {
			document.removeEventListener('visibilitychange', onVisibility);
			cancel();
		};
	});
}

/** Calls `run` every `period` ms while the tab is visible, for what changes without an activity entry (a scheduled check
 *  that confirms a result moves only its time). A hidden tab keeps no timer; when it is shown again, `run` is called at
 *  once if a period has passed since the last call, else when that period ends. Call during component initialisation;
 *  the timer and the listener go with the component. */
export function everyVisible(run: () => unknown, period: number): void {
	let last = Date.now();
	let timer: ReturnType<typeof setTimeout> | undefined;
	const schedule = () => {
		clearTimeout(timer);
		timer = undefined;
		if (document.visibilityState === 'hidden') return;
		if (Date.now() - last >= period) {
			last = Date.now();
			run();
		}
		timer = setTimeout(schedule, last + period - Date.now());
	};
	$effect(() => {
		schedule();
		document.addEventListener('visibilitychange', schedule);
		return () => {
			document.removeEventListener('visibilitychange', schedule);
			clearTimeout(timer);
			timer = undefined;
		};
	});
}

const MINUTE_MS = 60_000;
let minuteNow = Date.now();
const minuteTicks = createSubscriber((update) => {
	minuteNow = Date.now();
	const timer = setInterval(() => {
		minuteNow = Date.now();
		update();
	}, MINUTE_MS);
	return () => clearInterval(timer);
});

/** One clock for every "N ago" on screen, ticked once a minute while something reads it in an effect or a template (one
 *  timer for the tab, stopped when the last reader goes), so an idle page shows its ages growing. Read outside an
 *  effect, it is the current time. */
export const minuteClock = {
	get now(): number {
		if (!$effect.tracking()) return Date.now();
		minuteTicks();
		return minuteNow;
	}
};
