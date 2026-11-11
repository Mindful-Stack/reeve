<script lang="ts">
	import { tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { AffectedStory, DecisionQueue, DecisionQueueEntry, QueueLane } from '#lib/api/types.ts';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import { filterDecisions, itemHref, LANE_ANCHOR, LANE_LABEL, LANE_ORDER } from '#lib/decisions.ts';
	import { decisionHref } from '#lib/drawer.ts';
	import { minuteClock, onLive } from '#lib/events.svelte.ts';
	import { age, errorMessage } from '#lib/format.ts';
	import { session } from '#lib/me.svelte.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';

	// The toggles live in the query string so they survive a reload: every decider unless ?mine=1 (the Tuesday sync sees
	// the whole queue; an old ?all=1 is ignored), the Closed group folded unless ?closed=1.
	const mine = $derived(page.url.searchParams.get('mine') === '1');
	const showClosed = $derived(page.url.searchParams.get('closed') === '1');

	let queue = $state<DecisionQueue | null>(null);
	let initiatives = $state<ReadonlySet<string>>(new Set());
	let epics = $state<ReadonlySet<string>>(new Set());
	let loadError = $state<string | null>(null);
	/** A refetch failed after the list had loaded: the list (and any unsent comment) stays mounted; the banner offers a retry. */
	let staleNotice = $state<string | null>(null);

	/** The lanes that sit beyond the two-sprint window: the Tuesday queue, below a divider. */
	const OUTSIDE = new Set<QueueLane>(['beyond', 'open', 'satisfied', 'closed']);
	const NEEDED = new Set<QueueLane>(['review', 'this_sprint', 'next_sprint']);

	/** In the server's order, never re-sorted: the server orders each group by urgency and blocked work. */
	const decisions = $derived((queue?.entries ?? []).map((e) => e.decision));
	const shown = $derived(filterDecisions(decisions, { mine: mine ? session.me?.email : undefined, closed: true }));
	const laneById = $derived(new Map<number, QueueLane>((queue?.entries ?? []).map((e) => [e.decision.id, e.lane])));
	const blocks = $derived(new Map<number, AffectedStory[]>((queue?.entries ?? []).map((e) => [e.decision.id, e.blocks])));
	const entries = $derived(new Map<number, DecisionQueueEntry>((queue?.entries ?? []).map((e) => [e.decision.id, e])));
	const groups = $derived(LANE_ORDER.map((id) => ({ id, folded: id === 'closed' && !showClosed, listId: `decisions-${id}` })));
	/** The lanes that have a row for the people shown (the Mine filter applies). */
	const populated = $derived(new Set(shown.map((d) => laneById.get(d.id)).filter((l): l is QueueLane => !!l)));
	/** The first lane below the window with a row: the divider goes before its heading. */
	const firstOutside = $derived(LANE_ORDER.find((l) => OUTSIDE.has(l) && populated.has(l)));
	/** Nothing is needed in the two-sprint window; said only when the queue has entries, since an empty queue says "No open decisions". */
	const nothingNeeded = $derived((queue?.entries.length ?? 0) > 0 && ![...NEEDED].some((l) => populated.has(l)));
	/** Beyond and Open are the lanes the Tuesday queue works through; with only Accepted or Closed below, the divider names just the window. */
	const tuesdayWork = $derived(populated.has('beyond') || populated.has('open'));
	/** The summary line, by lane: the server's counts for the window lanes and the overdue ones, and Beyond counted from the
	 *  entries (the server's summary has no field for it). A zero part is left out. */
	const summaryParts = $derived.by(() => {
		const s = queue?.summary;
		if (!s) return [];
		const beyond = (queue?.entries ?? []).filter((e) => e.lane === 'beyond').length;
		return [
			[s.review, 'to review'],
			[s.this_sprint, 'this sprint'],
			[s.next_sprint, 'next sprint'],
			[beyond, 'after next sprint'],
			[s.overdue, 'overdue']
		]
			.filter(([n]) => (n as number) > 0)
			.map(([n, what]) => `${n} ${what}`);
	});
	/** A link to a lane (#review, #this-sprint, #next-sprint) whose lane has no row has nothing to scroll to; say so. */
	const emptyLane = $derived.by(() => {
		const lane = (Object.keys(LANE_ANCHOR) as QueueLane[]).find((l) => LANE_ANCHOR[l] === page.url.hash.slice(1));
		return !!queue && !!lane && !populated.has(lane);
	});

	/** The newest check of any listed record: how fresh the whole page's statuses are. */
	const lastChecked = $derived(
		shown.reduce<string | null>((max, d) => {
			if (!d.record || !d.checked_at || Number.isNaN(Date.parse(d.checked_at))) return max;
			return !max || Date.parse(d.checked_at) > Date.parse(max) ? d.checked_at : max;
		}, null)
	);

	let latest = 0;

	// The lanes render after the first fetch, so a #review / #this-sprint / #next-sprint link scrolls once, then.
	let scrolled = false;
	$effect(() => {
		if (!queue || scrolled) return;
		scrolled = true;
		const id = page.url.hash.slice(1);
		if (id) void tick().then(() => document.getElementById(id)?.scrollIntoView?.());
	});

	/** The UTC date of a time: the server's overdue ordering and summary are by it. */
	const utcDay = (t: number) => new Date(t).toISOString().slice(0, 10);
	/** The UTC date the last reload asked on (not reactive). */
	let fetchedDay = utcDay(Date.now());

	async function reload() {
		const seq = ++latest;
		fetchedDay = utcDay(Date.now());
		try {
			const [q, i, e] = await Promise.all([
				unwrap(api.GET('/api/v1/decisions/queue', { params: { query: { include_closed: true } } })),
				unwrap(api.GET('/api/v1/initiatives')),
				unwrap(api.GET('/api/v1/epics'))
			]);
			if (seq !== latest) return; // a later toggle's reload owns the list
			queue = q;
			initiatives = new Set(i.map((x) => x.key));
			epics = new Set(e.map((x) => x.key));
			loadError = null;
			staleNotice = null;
		} catch (e) {
			if (seq !== latest) return;
			const m = errorMessage(e);
			if (queue) staleNotice = `Could not reload the decisions: ${m}`;
			else loadError = m;
		}
	}

	// Closed decisions are always fetched and only folded unless ?closed=1: a decision closed from this view (or by a
	// colleague under an open form) stays in the list, so its row state survives and the fold can keep it in sight.
	$effect(() => {
		reload();
	});

	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
	onLive(reload);

	// A new UTC day moves overdue decisions up and changes the summary with no activity entry, so the minute clock that
	// ages the page also refetches it then; a hidden tab waits until it is shown again.
	const visible = () => document.visibilityState !== 'hidden';
	function onNewDay(now: number) {
		if (visible() && utcDay(now) !== fetchedDay) void reload();
	}
	$effect(() => {
		onNewDay(minuteClock.now);
	});

	function setFlag(name: string, on: boolean) {
		const url = new URL(page.url.href);
		if (on) url.searchParams.set(name, '1');
		else url.searchParams.delete(name);
		url.searchParams.delete('all');
		return goto(url, { replace: true, reset: false });
	}
</script>

<svelte:head><title>Decisions · Reeve</title></svelte:head>
<svelte:document onvisibilitychange={() => onNewDay(Date.now())} />

<PageHeader title="Decisions">
	<Toggle
		label="Decider"
		options={[
			{ value: 'all', label: 'All' },
			{ value: 'mine', label: 'Mine' }
		]}
		value={mine ? 'mine' : 'all'}
		onchange={(v) => setFlag('mine', v === 'mine')}
	/>
	<label class="closed-toggle">
		<input type="checkbox" checked={showClosed} onchange={(e) => setFlag('closed', e.currentTarget.checked)} />
		Show closed
	</label>
	{#if lastChecked}
		<time class="checked-all muted" datetime={lastChecked} title={new Date(lastChecked).toLocaleString()}>Records checked {age(lastChecked, minuteClock.now)} ago</time>
	{/if}
</PageHeader>

{#if loadError}
	<Notice tone="error" onretry={reload}>Could not load the decisions: {loadError}</Notice>
{:else if !queue}
	<p class="muted">Loading…</p>
{:else}
	{#if staleNotice}
		<Notice tone="error" onretry={reload} ondismiss={() => (staleNotice = null)}>{staleNotice}</Notice>
	{/if}
	{#if summaryParts.length}
		<p class="summary">{summaryParts.join(' · ')}</p>
	{/if}
	{#if emptyLane}
		<p class="lane-note muted" data-testid="empty-lane">Nothing needed in this lane right now.</p>
	{/if}
	{#if nothingNeeded}
		<p class="quiet muted">{mine ? 'Nothing of yours needed' : 'Nothing needed'} this sprint or next.</p>
	{/if}
	<!-- The headings carry their own margins, so the list adds no gap between them and the groups. -->
	<DecisionList
		--decision-list-row-gap="0"
		decisions={shown}
		{blocks}
		{entries}
		decisionHref={(id) => decisionHref(page.url, id)}
		itemHref={(key) => itemHref(key, initiatives, epics)}
		onchanged={reload}
		groupOf={(d) => laneById.get(d.id)!}
		{groups}
		empty="No open decisions"
	>
		{#snippet heading(group, count)}
			{#if group === firstOutside}
				<p class="divider muted">Outside the two-sprint window{tuesdayWork ? ' · for the Tuesday queue' : ''}</p>
			{/if}
			<h2 id={LANE_ANCHOR[group as QueueLane]}>
				{#if group === 'closed'}
					<button
						type="button"
						class="ghost fold"
						aria-expanded={showClosed}
						aria-controls={showClosed ? 'decisions-closed' : undefined}
						onclick={() => setFlag('closed', !showClosed)}
						><span aria-hidden="true">{showClosed ? '▾' : '▸'}</span> {LANE_LABEL[group as QueueLane]} <span class="count">{count}</span></button
					>
				{:else}
					{LANE_LABEL[group as QueueLane]} <span class="count">{count}</span>
				{/if}
			</h2>
		{/snippet}
	</DecisionList>
{/if}

<style>
	h2 {
		margin: 20px 0 8px;
	}
	.summary {
		margin: 0 0 4px;
	}
	.quiet,
	.lane-note {
		margin: 0 0 4px;
	}
	.divider {
		margin: 24px 0 0;
		padding-top: 8px;
		border-top: 1px solid var(--line);
	}
	.divider + h2 {
		margin-top: 8px;
	}
	.fold {
		font: inherit;
	}
	.closed-toggle {
		display: inline-flex;
		gap: 6px;
		align-items: center;
	}
	.checked-all {
		margin-left: auto;
	}
</style>
