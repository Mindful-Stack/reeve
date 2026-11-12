<!-- web/src/lib/components/TriageItem.svelte: the triage page's item pane. It shows the item and edits its draft (bound);
	the page owns every request, the keys and the notice. Picking an epic moves focus on to Criteria. -->
<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Epic, Story } from '#lib/api/types.ts';
	import { COLUMN_LABEL, KINDS } from '#lib/board.ts';
	import Brief from '#lib/components/Brief.svelte';
	import { keyed } from '#lib/picker.ts';
	import type { Draft } from '#lib/triage.ts';
	import Picker from '#lib/ui/Picker.svelte';

	let {
		item,
		draft = $bindable(),
		epics,
		disabled,
		gone,
		open,
		onplace,
		armed,
		ondrop,
		onskip,
		similar
	}: {
		item: Story;
		draft: Draft;
		epics: Epic[];
		/** A write is in flight, or the item left the queue. */
		disabled: boolean;
		/** Where the item went when it left the queue meanwhile; null while it is still here. */
		gone: string | null;
		/** The item in the story drawer, over this page (`o`). */
		open: string;
		onplace: () => void;
		/** The armed drop's words for this item, or null. */
		armed: string | null;
		ondrop: () => void;
		onskip: () => void;
		/** The similar-items panel, between the description and the form. */
		similar: Snippet;
	} = $props();

	let criteriaEl: HTMLTextAreaElement | undefined = $state();
	const active = $derived(item.delivery !== 'todo');
</script>

<section class="item" aria-labelledby="triage-item-title">
	<h2 id="triage-item-title" tabindex="-1">
		<span class="key">{item.key}</span> {item.title}
	</h2>
	<p class="meta">
		{item.kind} · {COLUMN_LABEL[item.delivery]}{#if item.owner}{' '}· {item.owner}{/if}
		· <a href={open} data-sveltekit-reset="false">Open in drawer <kbd>o</kbd></a>
	</p>
	{#if active}
		<p class="warn">{COLUMN_LABEL[item.delivery]}{#if item.owner}, owned by {item.owner}{/if}: dropping it stops work.</p>
	{/if}
	{#if gone}<p class="gone" role="note">{gone}</p>{/if}
	{#if item.description.trim()}<div class="description"><Brief markdown={item.description} /></div>{/if}
	{@render similar()}
	<form class="place" aria-label="Place {item.key}" onsubmit={(e) => { e.preventDefault(); onplace(); }}>
		<div class="field epic">
			<span class="name" aria-hidden="true">Epic</span>
			<Picker label="Epic" options={keyed(epics)} bind:value={draft.fields.epic} placeholder="Search epics (e)" {disabled} onchange={() => criteriaEl?.focus()} />
		</div>
		<label class="field">Kind <span class="hint">(t)</span>
			<select bind:value={draft.fields.kind} {disabled}>
				{#each KINDS as k (k)}<option value={k}>{k}</option>{/each}
			</select>
		</label>
		<label class="field">Title <input name="title" bind:value={draft.fields.title} {disabled} /></label>
		<label class="field">Criteria, one per line <span class="hint">(c)</span>
			<textarea class="criteria" rows="4" bind:this={criteriaEl} bind:value={draft.fields.criteria} {disabled}></textarea>
		</label>
		<div class="actions">
			<button type="submit" class="primary" {disabled}>Place <kbd>Ctrl+Enter</kbd></button>
			<button type="button" class="danger" {disabled} onclick={ondrop} onkeydown={(e) => { if (e.repeat) e.preventDefault(); }}>{armed ?? 'Drop'} <kbd>d d</kbd></button>
			<button type="button" onclick={onskip}>Skip <kbd>s</kbd></button>
		</div>
	</form>
</section>

<style>
	.item { display: grid; gap: 12px; min-width: 0; }
	/* Focused on a phone (no queue list), it scrolls into view below the sticky notice, not behind it. */
	h2 { scroll-margin-top: 4em; }
	.meta { color: var(--muted); }
	.warn, .gone { color: var(--danger); }
	.field { display: grid; gap: 4px; }
	.hint, kbd { color: var(--muted); font-size: 0.85em; }
	.criteria { width: 100%; }
	.actions { display: flex; gap: 8px; flex-wrap: wrap; }
	/* One button per row on a phone: the armed Drop's longer words grow it in place instead of moving it to another row
	   under the second tap. */
	@media (max-width: 719px) { .actions { display: grid; } }
</style>
