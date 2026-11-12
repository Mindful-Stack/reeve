<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { ActivityEntry } from '#lib/api/types.ts';
	import { actionLabel, age, errorMessage, short } from '#lib/format.ts';
	import { storyHref } from '#lib/drawer.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import { onLive } from '#lib/events.svelte.ts';

	const LIMIT = 200;

	let entries = $state<ActivityEntry[] | null>(null);
	let loadError = $state<string | null>(null);

	// The toggle lives in the query string so it survives a reload and can be linked to.
	const overridesOnly = $derived(page.url.searchParams.get('overrides') === '1');

	function setOverridesOnly(on: boolean) {
		const url = new URL(page.url.href);
		if (on) url.searchParams.set('overrides', '1');
		else url.searchParams.delete('overrides');
		goto(url, { replace: true, reset: false });
	}

	let generation = 0;
	async function reload(only: boolean) {
		const mine = ++generation;
		try {
			// Newest first: the server lists activity in descending order.
			const list = await unwrap(
				api.GET('/api/v1/activity', {
					params: { query: only ? { limit: LIMIT, action: 'override' } : { limit: LIMIT } }
				})
			);
			if (mine !== generation) return; // a later toggle already asked for something else
			entries = list;
			loadError = null;
		} catch (e) {
			if (mine === generation) loadError = errorMessage(e);
		}
	}

	$effect(() => {
		reload(overridesOnly);
	});
	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
	onLive(() => reload(overridesOnly));

	/** Stories open in the drawer on this page; initiatives and epics have their own pages; anything else is plain text. */
	function entityHref(e: ActivityEntry): string | null {
		if (e.entity_type === 'story') return storyHref(page.url, e.entity_key);
		if (e.entity_type === 'initiative') return `/initiatives/${encodeURIComponent(e.entity_key)}`;
		if (e.entity_type === 'epic') return `/epics/${encodeURIComponent(e.entity_key)}`;
		return null;
	}

	/** The free-text reason an override carries in its payload, if any. */
	function reason(e: ActivityEntry): string | null {
		const p = e.payload;
		if (p && typeof p === 'object' && 'reason' in p) {
			const r = (p as { reason: unknown }).reason;
			if (typeof r === 'string' && r.trim()) return r;
		}
		return null;
	}

	/** The entry's action in words: `actionLabel`'s, without the key, which the row shows beside it. */
	function label(e: ActivityEntry): string {
		return actionLabel(e.action, e.payload);
	}
</script>

<svelte:head><title>Activity · Reeve</title></svelte:head>

<PageHeader title="Activity" />

<label class="toggle">
	<input
		type="checkbox"
		checked={overridesOnly}
		onchange={(ev) => setOverridesOnly(ev.currentTarget.checked)}
	/>
	Overrides only
</label>

{#if loadError}
	<Notice tone="error" onretry={() => reload(overridesOnly)}>Could not load activity: {loadError}</Notice>
{:else if entries === null}
	<p class="muted">Loading…</p>
{:else if entries.length === 0}
	<p class="muted">{overridesOnly ? 'No overrides yet.' : 'No activity yet.'}</p>
{:else}
	<ol class="rows">
		{#each entries as e (e.id)}
			{@const href = entityHref(e)}
			{@const why = e.action === 'override' ? reason(e) : null}
			<li data-testid="activity-{e.id}">
				<span class="age" title={e.at}>{age(e.at)}</span>
				<span class="actor" title={e.actor}>{short(e.actor)}</span>
				<span class="via via-{e.via}" title="via {e.via}">{e.via}</span>
				<span class="action">{label(e)}</span>
				<span class="entity">
					{e.entity_type}
					{#if href}
						<a {href}>{e.entity_key}</a>
					{:else}
						{e.entity_key}
					{/if}
				</span>
				{#if why}
					<q class="reason">{why}</q>
				{/if}
			</li>
		{/each}
	</ol>
	{#if entries.length === LIMIT}
		<p class="muted">Showing the latest {LIMIT} entries.</p>
	{/if}
{/if}

<style>
	.toggle {
		display: inline-flex;
		gap: 0.4rem;
		align-items: center;
		margin-bottom: 1rem;
	}
	.age {
		min-width: 2.5rem;
		color: var(--muted);
		font-variant-numeric: tabular-nums;
	}
	.actor {
		font-weight: 600;
	}
	.via {
		padding: 0 0.4rem;
		border: 1px solid transparent;
		border-radius: 999px;
		background: var(--lighter-background);
		color: var(--muted);
		font-size: 13px;
	}
	.via-automation {
		border-color: var(--accent);
		color: var(--accent);
	}
	.entity {
		color: var(--muted);
	}
	.reason {
		flex-basis: 100%;
		padding-left: 0;
		font-style: italic;
	}
</style>
