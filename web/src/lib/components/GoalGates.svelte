<!-- web/src/lib/components/GoalGates.svelte: what a goal's work waits on, derived on read by the server
     (`GET /sprints/planning`): the initiatives and epics among its items without a discovery mark, and its open required
     decisions that are not satisfied, each with what it needs (kb/ADR-0018 R3), then how many of each are done. These
     are not sprint-membership counts, so the panel's rule line (kb/ADR-0014 R2) is not repeated here. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { GoalGates, Reason } from '#lib/api/types.ts';
	import { shortLabel } from '#lib/board.ts';
	import { decisionHref } from '#lib/drawer.ts';
	import { itemRoute } from '#lib/urgency.ts';

	let {
		gates,
		linked
	}: {
		gates: GoalGates;
		/** Whether the goal links any item. */
		linked: boolean;
	} = $props();

	/** The decision a gate's reason names: every decision gate carries a `decision_*` reason. */
	const decisionOf = (r: Reason): { id: number; title: string } | null =>
		'decision_id' in r ? { id: r.decision_id, title: r.title } : null;
	/** The decision gates that name a decision, with it; the server always names one, so a gate without is left out. */
	const decisions = $derived(
		gates.decisions.flatMap((g) => {
			const ref = decisionOf(g.reason);
			return ref ? [{ g, ref }] : [];
		})
	);
	const waiting = $derived(gates.discoveries.length + decisions.length > 0);
	/** "Discovery m of n marked · decisions d of n decided"; a half whose n is 0 is left out. */
	const line = $derived.by(() => {
		const parts: string[] = [];
		if (gates.discovery_total)
			parts.push(`discovery ${gates.discovery_total - gates.discoveries.length} of ${gates.discovery_total} marked`);
		if (gates.decisions_total)
			parts.push(`decisions ${gates.decisions_total - gates.decisions.length} of ${gates.decisions_total} decided`);
		const s = parts.join(' · ');
		return s.charAt(0).toUpperCase() + s.slice(1);
	});
</script>

<div class="gates" data-testid="gates">
	<h5>Waits on</h5>
	{#if !linked}
		<p class="muted">No linked items yet</p>
	{:else}
		{#if waiting}
			<ul>
				{#each gates.discoveries as d (d.key)}
					<li data-testid="discovery-{d.key}">
						<a href={itemRoute(d, page.url)}><span class="key">{d.key}</span> {d.title}</a>
						<span class="muted">{d.item} discovery</span>
					</li>
				{/each}
				{#each decisions as { g, ref } (ref.id)}
					<!-- One link for the id and title, as DecisionList links a decision's title; the parts apart by "·". -->
					<li data-testid="decision-{ref.id}">
						<a data-sveltekit-reset="false" href={decisionHref(page.url, ref.id)}>#{ref.id} {ref.title}</a> ·
						<span class="small">{shortLabel(g.reason)}</span> ·
						<span class="muted">on {g.on}</span>
					</li>
				{/each}
			</ul>
		{:else}
			<p>Nothing waits</p>
		{/if}
		{#if line}<p class="muted" data-testid="gate-counts">{line}</p>{/if}
	{/if}
</div>

<style>
	.gates {
		display: grid;
		gap: 4px;
		padding-top: 6px;
		border-top: 1px solid var(--line);
	}
	h5 {
		margin: 0;
		font-size: 13px;
	}
	p {
		margin: 0;
	}
	ul {
		margin: 0;
		padding-left: 18px;
	}
</style>
