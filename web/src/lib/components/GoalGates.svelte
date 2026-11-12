<!-- web/src/lib/components/GoalGates.svelte: what a goal's work waits on, derived on read by the server
     (`GET /sprints/planning`): the initiatives and epics among its items without a discovery mark, and its open required
     decisions that are not satisfied, each with what it needs (kb/ADR-0018 R3), then how many of each are done. These
     are not sprint-membership counts, so the panel's rule line (kb/ADR-0014 R2) is not repeated here. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { GoalGates } from '#lib/api/types.ts';
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

	const waiting = $derived(gates.discoveries.length + gates.decisions.length > 0);
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
						<span class="muted">{d.item} in discovery</span>
					</li>
				{/each}
				{#each gates.decisions as g (('decision_id' in g.reason ? g.reason.decision_id : 0) + g.on)}
					{@const r = g.reason}
					<!-- Every decision gate carries a `decision_*` reason; the server always names the decision. -->
					{#if 'decision_id' in r}
						<li data-testid="decision-{r.decision_id}">
							<a data-sveltekit-reset="false" href={decisionHref(page.url, r.decision_id)}>#{r.decision_id} {r.title}</a> ·
							<span class="small">{shortLabel(r).replace(/^decision:? /, '')}</span> ·
							<span class="muted">on {g.on}</span>
						</li>
					{/if}
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
