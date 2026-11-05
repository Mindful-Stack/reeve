<!-- web/src/lib/components/NudgeBar.svelte: the start nudge. A small bar on a card that just started and serves no
     goal: picking a goal links it, "Upkeep" (or Escape, or ignoring it) only dismisses. Not a dialog: no focus trap, and the
     move it follows has already landed (kb/ADR-0014 R4). -->
<script lang="ts">
	import type { Goal } from '#lib/api/types.ts';
	import { truncate } from '#lib/sprint.ts';

	let {
		goals,
		busy = false,
		onpick,
		ondismiss
	}: {
		goals: Pick<Goal, 'id' | 'outcome'>[];
		busy?: boolean;
		onpick: (goalId: number) => void;
		ondismiss: () => void;
	} = $props();

	const id = $props.id();

	/** Escape dismisses, focus on the bar or not (focus follows the moved card's menu), unless it belongs to something
	 *  else: anything that already handled it, a dialog, or a form field (whose Escape cancels its own edit). */
	function keydown(e: KeyboardEvent) {
		if (e.key !== 'Escape' || e.defaultPrevented) return;
		const t = e.target;
		if (t instanceof Element && t.closest('dialog, input, textarea, select, [contenteditable]')) return;
		ondismiss();
	}
</script>

<svelte:window onkeydown={keydown} />

<div class="nudge" role="group" aria-labelledby="{id}-ask" data-testid="nudge">
	<span id="{id}-ask" class="ask" aria-live="polite">Serving which goal?</span>
	{#each goals as g (g.id)}
		<button type="button" title={g.outcome.length > 40 ? g.outcome : undefined} disabled={busy} onclick={() => onpick(g.id)}
			>{truncate(g.outcome, 40)}</button
		>
	{/each}
	<button type="button" class="ghost" onclick={ondismiss}>Upkeep</button>
</div>

<style>
	.nudge {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 6px;
		margin-top: 4px;
		padding: 6px 8px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
		font-size: 13px;
	}
	.ask {
		color: var(--muted);
	}
	button {
		height: auto;
		padding: 2px 8px;
		font-size: 13px;
	}
</style>
