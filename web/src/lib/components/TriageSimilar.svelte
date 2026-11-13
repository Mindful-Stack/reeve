<!-- web/src/lib/components/TriageSimilar.svelte: the current item's similar stories (the item itself left out), each with
	Inspect (the story drawer, over the page) and a Duplicate toggle: this item is a duplicate of that hit. A duplicate the
	list missed is handled in the drawer. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { SimilarStory } from '#lib/api/types.ts';
	import { hitState } from '#lib/board.ts';
	import { storyHref } from '#lib/drawer.ts';

	let {
		hits,
		marked,
		onmark
	}: {
		/** An empty list is also what a title too short (or too long) to compare gets: nothing was asked. */
		hits: SimilarStory[];
		/** The key this item is marked a duplicate of, or null. */
		marked: string | null;
		onmark: (key: string) => void;
	} = $props();
</script>

<section class="similar" aria-labelledby="triage-similar-heading">
	<h3 id="triage-similar-heading">Similar</h3>
	{#if !hits.length}
		<p class="muted">Nothing similar.</p>
	{:else}
		<ol>
			{#each hits as h, i (h.key)}
				<li class:marked={marked === h.key}>
					<span class="n" aria-hidden="true">{i + 1}</span>
					<span class="key">{h.key}</span> {h.title}
					<span class="state">{hitState(h)}</span>
					<a href={storyHref(page.url, h.key)} data-sveltekit-reset="false">Inspect {h.key}</a>
					<button type="button" aria-pressed={marked === h.key} onclick={() => onmark(h.key)}>
						Duplicate of this ({i + 1})
					</button>
				</li>
			{/each}
		</ol>
	{/if}
</section>

<style>
	ol { padding-left: 0; list-style: none; display: grid; gap: 4px; }
	li { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
	li.marked { outline: 2px solid var(--danger); border-radius: 4px; }
	.state, .muted, .n { color: var(--muted); }
</style>
