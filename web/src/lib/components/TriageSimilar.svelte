<!-- web/src/lib/components/TriageSimilar.svelte: the current item's similar stories (the item itself left out), each with
	Inspect (the story drawer, over the page) and a Duplicate toggle. A duplicate the list missed is handled in the drawer. -->
<script lang="ts">
	import type { SimilarStory } from '#lib/api/types.ts';
	import { COLUMN_LABEL } from '#lib/board.ts';

	let {
		hits,
		marked,
		inspect,
		onmark
	}: {
		/** null: the title is outside the 8..200 characters the server compares, so nothing was asked. */
		hits: SimilarStory[] | null;
		/** The key marked as the duplicate, or null. */
		marked: string | null;
		inspect: (key: string) => string;
		onmark: (key: string) => void;
	} = $props();
</script>

<section class="similar" aria-label="Similar">
	<h3>Similar</h3>
	{#if hits === null}
		<p class="muted">Too short to compare.</p>
	{:else if !hits.length}
		<p class="muted">Nothing similar.</p>
	{:else}
		<ol>
			{#each hits as h, i (h.key)}
				<li class:marked={marked === h.key}>
					<span class="n" aria-hidden="true">{i + 1}</span>
					<span class="key">{h.key}</span> {h.title}
					<span class="state">{COLUMN_LABEL[h.delivery]}{#if h.delivery === 'done' && h.resolution}{' '}· {h.resolution}{/if}</span>
					<a href={inspect(h.key)} data-sveltekit-reset="false">Inspect {h.key}</a>
					<button type="button" aria-pressed={marked === h.key} onclick={() => onmark(h.key)}>
						Duplicate ({i + 1})
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
