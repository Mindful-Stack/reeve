<!-- web/src/lib/components/GoalLinkButton.svelte: while the header picker is linking, links or unlinks one initiative or
     epic to the picked goal ("+ Link" / "✓ Linked"; the goal is named in the header, the tooltip and the accessible
     name); renders nothing otherwise. A link is a link, never a parent or progress (kb/ADR-0014 R3). -->
<script lang="ts">
	import { linkingGoal, picker, toggleLink } from '#lib/goalPicker.svelte.ts';

	let { key }: { key: string } = $props();

	const goal = $derived(linkingGoal());
	const linked = $derived(goal?.links.includes(key) ?? false);
</script>

{#if goal}
	<button
		type="button"
		class="goal-link"
		class:linked
		aria-pressed={linked}
		aria-label="Link {key} to goal: {goal.outcome}"
		title={linked ? `Linked to: ${goal.outcome} (click to unlink)` : `Link to: ${goal.outcome}`}
		aria-disabled={picker.writing}
		onclick={() => toggleLink(key)}>{linked ? '✓ Linked' : '+ Link'}</button
	>
{/if}

<style>
	.goal-link {
		justify-self: start;
		height: auto;
		padding: 2px 10px;
		border: 1px dashed var(--accent);
		border-radius: 999px;
		background: transparent;
		font-size: 0.85em;
		white-space: nowrap;
	}
	.goal-link[aria-disabled='true'] {
		opacity: 0.5;
		cursor: progress;
	}
	.goal-link.linked {
		border-style: solid;
		background: var(--accent);
		color: var(--on-accent);
	}
	.goal-link.linked:hover:not([aria-disabled='true']) {
		background: color-mix(in srgb, var(--accent) 88%, var(--foreground));
	}
</style>
