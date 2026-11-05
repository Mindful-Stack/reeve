<!-- web/src/lib/components/GoalsStrip.svelte: the active sprint's goals above the board, each a filter. -->
<script lang="ts">
	import type { SprintView } from '#lib/api/types.ts';
	import { confidenceLabel, confidenceTone, RULE, upkeepText } from '#lib/sprint.ts';

	let {
		view,
		selected,
		onselect
	}: {
		/** The active sprint's view, or null: the strip renders nothing without an active sprint. */
		view: SprintView | null;
		/** The board's goal filter: a goal's id, `'none'` (upkeep) or null. */
		selected: number | 'none' | null;
		/** A chip's filter, or null when the selected chip is clicked again. */
		onselect: (goal: number | 'none' | null) => void;
	} = $props();

	const progress = $derived(new Map(view?.counts.goals.map((g) => [g.goal_id, g]) ?? []));
	const pick = (g: number | 'none') => onselect(selected === g ? null : g);
</script>

{#if view && view.sprint.state === 'active'}
	<div class="strip" role="group" aria-label="Sprint goals" title={RULE} data-testid="goals-strip">
		{#each view.sprint.goals as g (g.id)}
			{@const p = progress.get(g.id)}
			<button
				type="button"
				class="chip"
				aria-pressed={selected === g.id}
				data-testid="goal-chip-{g.id}"
				onclick={() => pick(g.id)}
			>
				<span
					class="dot {confidenceTone(g.confidence)}"
					role="img"
					aria-label="confidence: {g.confidence ? confidenceLabel(g.confidence) : 'not recorded'}"
				></span>
				<span class="outcome">{g.outcome}</span>
				<span class="counts"
					><span class="visually-hidden">completed / in flight / todo: </span>{p?.completed ?? 0} / {p?.in_flight ?? 0} / {p?.todo ??
						0}</span
				>
			</button>
		{/each}
		<button
			type="button"
			class="chip"
			aria-pressed={selected === 'none'}
			data-testid="goal-chip-upkeep"
			title={upkeepText(view.counts, view.upkeep_line)}
			onclick={() => pick('none')}>Upkeep</button
		>
		<!-- kb/ADR-0014 R2: every view that shows counts states the membership rule. -->
		<p class="rule">{RULE}</p>
	</div>
{/if}

<style>
	.strip {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		margin: 0 0 10px;
	}
	.rule {
		flex-basis: 100%;
		margin: 0;
		color: var(--muted);
		font-size: 12px;
	}
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		max-width: 100%;
		height: auto;
		padding: 4px 10px;
		border: 1px solid var(--line-control);
		border-radius: 999px;
		background: var(--raised);
		color: var(--foreground);
		font-size: 13px;
		cursor: pointer;
	}
	.chip[aria-pressed='true'] {
		border-color: var(--accent);
		box-shadow: inset 0 0 0 1px var(--accent);
	}
	.outcome {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.counts {
		color: var(--muted);
		font-variant-numeric: tabular-nums;
		white-space: nowrap;
	}
	.dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--muted);
	}
	.ready {
		background: var(--ready);
	}
	.warn {
		background: var(--warn);
	}
	.danger {
		background: var(--danger);
	}
</style>
