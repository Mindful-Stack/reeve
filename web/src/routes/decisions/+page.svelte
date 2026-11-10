<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { AffectedStory, DecisionQueue, QueueGroup } from '#lib/api/types.ts';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import { filterDecisions, itemHref } from '#lib/decisions.ts';
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

	/** The queue's groups in their render order (the server's `QueueGroup` order), with what each heading says. */
	const LABEL: Record<QueueGroup, string> = {
		blocking: 'Blocking work',
		open: 'Open, not blocking yet',
		satisfied: 'Accepted',
		closed: 'Closed'
	};
	const ORDER: QueueGroup[] = ['blocking', 'open', 'satisfied', 'closed'];

	/** In the server's order, never re-sorted: the server orders each group by urgency and blocked work. */
	const decisions = $derived((queue?.entries ?? []).map((e) => e.decision));
	const shown = $derived(filterDecisions(decisions, { mine: mine ? session.me?.email : undefined, closed: true }));
	const groupById = $derived(new Map<number, QueueGroup>((queue?.entries ?? []).map((e) => [e.decision.id, e.group])));
	const blocks = $derived(new Map<number, AffectedStory[]>((queue?.entries ?? []).map((e) => [e.decision.id, e.blocks])));
	const groups = $derived(ORDER.map((id) => ({ id, folded: id === 'closed' && !showClosed, listId: `decisions-${id}` })));
	const summary = $derived(queue?.summary);

	/** The newest check of any listed record: how fresh the whole page's statuses are. */
	const lastChecked = $derived(
		shown.reduce<string | null>((max, d) => {
			if (!d.record || !d.checked_at || Number.isNaN(Date.parse(d.checked_at))) return max;
			return !max || Date.parse(d.checked_at) > Date.parse(max) ? d.checked_at : max;
		}, null)
	);

	let latest = 0;

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
	{#if summary}
		<p class="summary">
			{summary.open} open · {summary.blocking} blocking {summary.blocked_stories}
			{summary.blocked_stories === 1 ? 'story' : 'stories'} · {summary.overdue} overdue
		</p>
	{/if}
	<!-- The headings carry their own margins, so the list adds no gap between them and the groups. -->
	<DecisionList
		--decision-list-row-gap="0"
		decisions={shown}
		{blocks}
		itemHref={(key) => itemHref(key, initiatives, epics)}
		onchanged={reload}
		groupOf={(d) => groupById.get(d.id)!}
		{groups}
		empty="No open decisions"
	>
		{#snippet heading(group, count)}
			<h2>
				{#if group === 'closed'}
					<button
						type="button"
						class="ghost fold"
						aria-expanded={showClosed}
						aria-controls={showClosed ? 'decisions-closed' : undefined}
						onclick={() => setFlag('closed', !showClosed)}
						><span aria-hidden="true">{showClosed ? '▾' : '▸'}</span> {LABEL[group as QueueGroup]} <span class="count">{count}</span></button
					>
				{:else}
					{LABEL[group as QueueGroup]} <span class="count">{count}</span>
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
