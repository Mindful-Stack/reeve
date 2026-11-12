<!-- The promise line on the story drawer and the epic page (kb/ADR-0025): who promised what to whom, both promises when a
     story has its own and its epic's, and the state, all as text. The epic's key links to the epic. -->
<script lang="ts">
	import type { EpicPromise, StoryPromise } from '#lib/api/types.ts';
	import { epicLineText, isEpic, lineParts, type LinePart } from '#lib/promise.ts';

	let { promise }: { promise: StoryPromise | EpicPromise } = $props();

	const parts = $derived<LinePart[]>(isEpic(promise) ? [{ text: epicLineText(promise) }] : lineParts(promise));
</script>

<p class="promise-line" data-testid="promise-line">
	{#each parts as p, i (i)}{#if 'epic' in p}<a href="/epics/{encodeURIComponent(p.epic)}">{p.epic}</a>{:else}{p.text}{/if}{/each}
</p>

<style>
	.promise-line {
		margin: 0;
		font-size: 14px;
	}
</style>
