<script lang="ts">
	import type { ActivityEntry } from '#lib/api/types.ts';
	import { actionLabel, age, short } from '#lib/format.ts';

	// An item's activity, newest first: who, what, how long ago. An override is highlighted with the reason given for it.
	let { entries }: { entries: ActivityEntry[] } = $props();

	const newestFirst = $derived([...entries].sort((a, b) => b.id - a.id));

	/** The free-text reason an override was logged with, when its payload carries one. */
	function overrideReason(e: ActivityEntry): string | null {
		const p = e.payload;
		if (e.action !== 'override' || !p || typeof p !== 'object' || !('reason' in p)) return null;
		return typeof p.reason === 'string' ? p.reason : null;
	}
</script>

{#if newestFirst.length === 0}
	<p class="muted">No activity.</p>
{:else}
	<ol class="activity">
		{#each newestFirst as e (e.id)}
			{@const why = overrideReason(e)}
			<li class:override={e.action === 'override'}>
				<span class="actor" title={e.actor}>{short(e.actor)}</span>
				<span class="action">{e.action.startsWith('attachment_') ? actionLabel(e.action, e.payload) : e.action}</span>
				{#if e.via !== 'interactive'}<span class="via">{e.via}</span>{/if}
				<span class="muted" title={new Date(e.at).toLocaleString()}>{age(e.at)}</span>
				{#if why}<p class="why">“{why}”</p>{/if}
			</li>
		{/each}
	</ol>
{/if}

<style>
	.activity {
		display: grid;
		gap: 0.3rem;
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: 14px;
	}
	li {
		display: flex;
		flex-wrap: wrap;
		gap: 0.2rem 0.5rem;
		align-items: baseline;
		padding: 0.15rem 0.4rem;
		border-radius: 4px;
	}
	.override {
		background: var(--warn-soft);
	}
	.action {
		font-weight: 600;
	}
	.via {
		padding: 0 0.3rem;
		border: 1px solid var(--line);
		border-radius: 3px;
		font-size: 13px;
	}
	.why {
		flex-basis: 100%;
		margin: 0;
		font-style: italic;
	}
	.muted {
		margin: 0;
		color: var(--muted);
	}
</style>
