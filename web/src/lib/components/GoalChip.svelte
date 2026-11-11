<!-- web/src/lib/components/GoalChip.svelte: the first goal the server says a story serves (kb/ADR-0014 R3), cut to 40
     characters, with every goal's outcome in its tooltip. The story card and Home's story row show it; the drawer lists
     them all. -->
<script lang="ts">
	import type { Story } from '#lib/api/types.ts';
	import { truncate } from '#lib/sprint.ts';

	let { goals }: { goals: Story['goals'] } = $props();
</script>

{#if goals[0]}<span class="goal" title={goals.map((g) => g.outcome).join('\n')}>goal: {truncate(goals[0].outcome, 40)}</span>{/if}

<style>
	.goal {
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
</style>
