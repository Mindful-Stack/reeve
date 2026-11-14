<script lang="ts">
	import { tick } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { DecisionQueue, DecisionQueueEntry } from '#lib/api/types.ts';
	import { COLUMN_LABEL } from '#lib/board.ts';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import Glyph from '#lib/components/Glyph.svelte';
	import NeededInReason from '#lib/components/NeededInReason.svelte';
	import { filterDecisions, itemHref, LANE_ANCHOR, LANE_LABEL, LANE_ORDER, queueRows, type DiscoveryRow, type PageLane, type QueueRow } from '#lib/decisions.ts';
	import { decisionHref } from '#lib/drawer.ts';
	import { minuteClock, onLive } from '#lib/events.svelte.ts';
	import { age, errorMessage, plural } from '#lib/format.ts';
	import { session } from '#lib/me.svelte.ts';
	import { itemRoute } from '#lib/urgency.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';

	// The readiness queue (spec GRA-611): one list of what holds work back. The toggles live in the query string so they
	// survive a reload: every decider unless ?mine=1 (an old ?all=1 is ignored), the Closed group folded unless ?closed=1.
	const mine = $derived(page.url.searchParams.get('mine') === '1');
	const showClosed = $derived(page.url.searchParams.get('closed') === '1');

	let queue = $state<DecisionQueue | null>(null);
	let initiatives = $state<ReadonlySet<string>>(new Set());
	let epics = $state<ReadonlySet<string>>(new Set());
	let loadError = $state<string | null>(null);
	/** A refetch failed after the list had loaded: the list (and any unsent comment) stays mounted; the banner offers a retry. */
	let staleNotice = $state<string | null>(null);

	/** The lanes no goal or promise needs: the divider goes before the first of them that has a row. */
	const OUTSIDE = new Set<PageLane>(['now', 'later', 'open', 'satisfied', 'closed']);

	/** A DecisionList group: a lane's own group (its heading, and its decisions before any discovery row), a discovery
	 *  row's group (its nested decisions), or a run of standalone decisions after a row. */
	type Group = { id: string; lane: PageLane; folded: boolean; listId: string; always?: boolean; row?: DiscoveryRow };

	const me = $derived(mine ? session.me?.email : undefined);
	/** In the server's order, never re-sorted. */
	const decisions = $derived((queue?.entries ?? []).map((e) => e.decision));
	const shown = $derived(filterDecisions(decisions, { mine: me, closed: true }));
	const entries = $derived(new Map<number, DecisionQueueEntry>((queue?.entries ?? []).map((e) => [e.decision.id, e])));
	/** The top-level rows per page lane, Mine applied (queueRows). */
	const lanes = $derived(queue ? queueRows(queue, { me }) : new Map<PageLane, QueueRow[]>());
	/** The groups, lane by lane. Each lane's own group comes first and is always rendered while the lane has rows, so its
	 *  heading leads the lane even when a discovery row comes first; it is kept when the lane is empty, so a decision
	 *  pinned in it while being edited or closed stays in sight under its heading. */
	const layout = $derived.by(() => {
		const groups: Group[] = [];
		const of = new Map<number, string>();
		for (const lane of LANE_ORDER) {
			const rows = lanes.get(lane) ?? [];
			const folded = lane === 'closed' && !showClosed;
			// Not `always` when folded: a folded Closed-only queue then shows the empty text instead of a bare heading.
			let run: Group = { id: lane, lane, folded, listId: `decisions-${lane}`, always: rows.length > 0 && !folded };
			groups.push(run);
			let after: string | null = null;
			for (const r of rows) {
				if (r.kind === 'discovery') {
					const id = `row:${r.row.key}`;
					groups.push({ id, lane, folded: false, listId: `decisions-row-${r.row.key}`, always: true, row: r });
					for (const e of r.nested) of.set(e.decision.id, id);
					after = r.row.key;
				} else {
					if (after) {
						// Keyed by the row it follows, so a pinned decision keeps its group when an earlier row leaves.
						run = { id: `${lane}-after-${after}`, lane, folded, listId: `decisions-${lane}-after-${after}` };
						groups.push(run);
						after = null;
					}
					of.set(r.entry.decision.id, run.id);
				}
			}
		}
		return { groups, of, byId: new Map(groups.map((g) => [g.id, g])) };
	});
	/** The lanes with a row for the people shown (the Mine filter applies). */
	const populated = $derived(new Set(LANE_ORDER.filter((l) => (lanes.get(l)?.length ?? 0) > 0)));
	const firstOutside = $derived(LANE_ORDER.find((l) => OUTSIDE.has(l) && populated.has(l)));
	/** Nothing is needed this sprint or next; said only when the queue has something, since an empty one says so itself. */
	const nothingNeeded = $derived(
		(queue?.entries.length ?? 0) + (queue?.discoveries.length ?? 0) > 0 && !populated.has('this_sprint') && !populated.has('next_sprint')
	);
	/** The server's counts (unfiltered by Mine); a zero part is left out. Now and Later carry their counts on their headings. */
	const summaryParts = $derived.by(() => {
		const s = queue?.summary;
		if (!s) return [];
		return (
			[
				[s.review, 'to review'],
				[s.this_sprint, 'this sprint'],
				[s.next_sprint, 'next sprint'],
				[s.overdue, 'overdue']
			] as const
		)
			.filter(([n]) => n > 0)
			.map(([n, what]) => `${n} ${what}`);
	});
	/** A link to a lane (#this-sprint, #next-sprint) whose lane has no row has nothing to scroll to; say so. */
	const emptyLane = $derived.by(() => {
		const lane = (Object.keys(LANE_ANCHOR) as PageLane[]).find((l) => LANE_ANCHOR[l] === page.url.hash.slice(1));
		return !!queue && !!lane && !populated.has(lane);
	});
	/** The discovery rows whose stories are listed. */
	const storiesOpen = new SvelteSet<string>();

	/** The newest check of any listed record: how fresh the whole page's statuses are. */
	const lastChecked = $derived(
		shown.reduce<string | null>((max, d) => {
			if (!d.record || !d.checked_at || Number.isNaN(Date.parse(d.checked_at))) return max;
			return !max || Date.parse(d.checked_at) > Date.parse(max) ? d.checked_at : max;
		}, null)
	);

	let latest = 0;

	// The lanes render after the first fetch, so a #this-sprint / #next-sprint link scrolls once, then.
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
			if (queue) staleNotice = `Could not reload the readiness queue: ${m}`;
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

<svelte:head><title>Readiness · Reeve</title></svelte:head>
<svelte:document onvisibilitychange={() => onNewDay(Date.now())} />

{#snippet discovery(r: DiscoveryRow)}
	{@const d = r.row}
	{@const held = d.blocks.length}
	{@const holds = held ? `Holds ${plural(held, 'story', 'stories')}${d.under_way.length ? ` · ${d.under_way.length} under way` : ''}` : `${d.under_way.length} under way`}
	{@const open = storiesOpen.has(d.key)}
	<div class="discovery" data-discovery={d.key}>
		<h3 class="cell headline"><a class="item" href={itemRoute({ item: d.level, key: d.key }, page.url)}><Glyph level={d.level} /><span class="key">{d.key}</span> <span class="title">{d.title}</span></a></h3>
		<span class="cell holds"
			><button
				type="button"
				class="ghost inline"
				aria-label="{holds}: {d.title}"
				aria-expanded={open}
				aria-controls={open ? `discovery-${d.key}-stories` : undefined}
				onclick={() => (open ? storiesOpen.delete(d.key) : storiesOpen.add(d.key))}>{holds} <span aria-hidden="true">{open ? '▾' : '▸'}</span></button
			></span
		>
		<span class="cell status"><StatusDot tone="warn" label={d.also ? `in discovery · with ${d.also}` : 'in discovery'} /></span>
		{#if d.needed_in_reason}<p class="reason" data-testid="reason"><NeededInReason reason={d.needed_in_reason} row={r} /></p>{/if}
		{#if open}
			<ul class="stories" id="discovery-{d.key}-stories">
				{#each d.blocks as s (s.key)}<li><a href={itemHref(s.key, initiatives, epics)}><span class="key">{s.key}</span> {s.title}</a></li>{/each}
				{#each d.under_way as s (s.key)}<li><a href={itemHref(s.key, initiatives, epics)}><span class="key">{s.key}</span> {s.title}</a> <span class="muted">· {COLUMN_LABEL[s.delivery]}, under way</span></li>{/each}
			</ul>
		{/if}
	</div>
{/snippet}

<PageHeader title="Readiness">
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
	<Notice tone="error" onretry={reload}>Could not load the readiness queue: {loadError}</Notice>
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
		{entries}
		inQueue
		decisionHref={(id) => decisionHref(page.url, id)}
		itemHref={(key) => itemHref(key, initiatives, epics)}
		onchanged={reload}
		groupOf={(d) => layout.of.get(d.id)!}
		groups={layout.groups}
		empty={mine ? 'Nothing of yours holds work back' : 'Nothing holds work back'}
	>
		{#snippet heading(id, count)}
			{@const g = layout.byId.get(id)!}
			{#if g.row}
				{@render discovery(g.row)}
			{:else if id === g.lane}
				<!-- `count` covers a decision pinned here while it is edited or closed, after its lane emptied. -->
				{@const n = lanes.get(g.lane)?.length || count}
				{#if g.lane === firstOutside}
					<p class="divider muted">No goal or promise needs these yet</p>
				{/if}
				<h2 id={LANE_ANCHOR[g.lane]}>
					{#if g.lane === 'closed'}
						<button
							type="button"
							class="ghost fold"
							aria-expanded={showClosed}
							aria-controls={showClosed ? 'decisions-closed' : undefined}
							onclick={() => setFlag('closed', !showClosed)}
							><span aria-hidden="true">{showClosed ? '▾' : '▸'}</span> {LANE_LABEL[g.lane]} <span class="count">{n}</span></button
						>
					{:else}
						{LANE_LABEL[g.lane]} <span class="count">{n}</span>
					{/if}
				</h2>
			{/if}
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
	/* A discovery row sits on the decisions' columns (subgrid): title, holds (column 4), status (column 6). */
	.discovery {
		display: grid;
		grid-template-columns: subgrid;
		row-gap: 2px;
		align-items: center;
		margin-top: 8px;
		padding: 6px 12px;
		border: 1px dashed var(--warn);
		border-radius: 6px;
		background: var(--warn-soft);
	}
	/* The row's heading (h3, so it heads its nested decisions for a screen reader) reads like a decision's title:
	   body size, the title bold. */
	.discovery .headline {
		grid-column: 1;
		margin: 0;
		font: inherit;
	}
	.discovery .title {
		font-weight: 600;
	}
	.discovery .holds {
		grid-column: 4;
	}
	.discovery .status {
		grid-column: 6;
	}
	.discovery .cell:not(.headline) {
		font-size: 14px;
	}
	.discovery .reason,
	.discovery .stories {
		grid-column: 1 / -1;
		margin: 0;
		font-size: 14px;
		color: var(--muted);
	}
	.discovery .stories {
		display: grid;
		gap: 2px;
		padding-left: 1rem;
	}
	.discovery .inline {
		padding: 0 0.25rem;
		font-size: 14px;
	}
	/* Its nested decisions carry the row's mark on their left edge. */
	:global(.decision-list ul[id^='decisions-row-'] > li) {
		border-left: 3px solid var(--warn);
	}
	@container (max-width: 72rem) {
		.discovery {
			display: flex;
			flex-wrap: wrap;
			gap: 2px 12px;
		}
		.discovery .headline,
		.discovery .reason,
		.discovery .stories {
			flex-basis: 100%;
		}
	}
</style>
