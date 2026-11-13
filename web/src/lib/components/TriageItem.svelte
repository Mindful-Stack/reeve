<!-- web/src/lib/components/TriageItem.svelte: the triage page's item pane. It shows the item and edits its draft (bound),
	with the similar stories between the description and the form; the page owns every request, the keys and the notice.
	Picking an epic moves focus on to Criteria. The page moves focus into the pane through `focus`. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { Epic, SimilarStory, Story } from '#lib/api/types.ts';
	import { COLUMN_LABEL, KINDS } from '#lib/board.ts';
	import Brief from '#lib/components/Brief.svelte';
	import TriageSimilar from '#lib/components/TriageSimilar.svelte';
	import { storyHref } from '#lib/drawer.ts';
	import { keyed, PICKER_INPUT } from '#lib/picker.ts';
	import { ACTIVE } from '#lib/sprint.ts';
	import type { Draft } from '#lib/triage.ts';
	import Picker from '#lib/ui/Picker.svelte';

	let {
		item,
		draft = $bindable(),
		epics,
		hits,
		disabled,
		gone,
		onplace,
		armed,
		ondrop,
		onskip,
		onmark
	}: {
		item: Story;
		draft: Draft;
		epics: Epic[];
		/** The item's similar stories, itself left out. */
		hits: SimilarStory[];
		/** A write is in flight, or the item left the queue. */
		disabled: boolean;
		/** Where the item went when it left the queue meanwhile; null while it is still here. */
		gone: string | null;
		onplace: () => void;
		/** The drop is armed: the next press confirms it. */
		armed: boolean;
		ondrop: () => void;
		onskip: () => void;
		/** A similar story's Duplicate toggle. */
		onmark: (key: string) => void;
	} = $props();

	let root: HTMLElement | undefined = $state();
	let headingEl: HTMLElement | undefined = $state();
	let criteriaEl: HTMLTextAreaElement | undefined = $state();
	// Work in progress is the only work a drop stops, and not once the item has left the queue (the form is disabled then).
	const active = $derived(!gone && ACTIVE.has(item.delivery));

	/** Moves focus to a part of the pane. */
	export function focus(part: 'epic' | 'criteria' | 'heading'): void {
		if (part === 'epic') root?.querySelector<HTMLElement>(PICKER_INPUT)?.focus();
		else if (part === 'criteria') criteriaEl?.focus();
		else headingEl?.focus();
	}
</script>

<section class="item" aria-labelledby="triage-item-title" bind:this={root}>
	<h2 id="triage-item-title" tabindex="-1" bind:this={headingEl}>
		<span class="key">{item.key}</span> {item.title}
	</h2>
	<p class="meta">
		{item.kind} · {COLUMN_LABEL[item.delivery]}{#if item.owner}{' '}· {item.owner}{/if}
		· <a href={storyHref(page.url, item.key)} data-sveltekit-reset="false">Open in drawer <kbd aria-hidden="true">o</kbd></a>
	</p>
	{#if active}
		<p class="warn">{COLUMN_LABEL[item.delivery]}{#if item.owner}, owned by {item.owner}{/if}: dropping it stops work.</p>
	{/if}
	{#if gone}<p class="gone" role="note">{gone}</p>{/if}
	{#if item.description.trim()}<div class="description"><Brief markdown={item.description} /></div>{/if}
	<TriageSimilar {hits} marked={draft.duplicateOf} {onmark} />
	<form class="place" aria-label="Place {item.key}" onsubmit={(e) => { e.preventDefault(); onplace(); }}>
		<label class="field epic">Epic <span class="hint">(e)</span>
			<Picker label="Epic" options={keyed(epics)} bind:value={draft.fields.epic} {disabled} onchange={() => criteriaEl?.focus()} />
		</label>
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
			<button type="submit" class="primary" {disabled}>Place <kbd aria-hidden="true">Ctrl+Enter</kbd></button>
			<button type="button" class="danger" {disabled} onclick={ondrop} onkeydown={(e) => { if (e.repeat) e.preventDefault(); }}>{armed ? 'Confirm drop' : 'Drop'} <kbd aria-hidden="true">d d</kbd></button>
			<button type="button" onclick={onskip}>Skip <kbd aria-hidden="true">s</kbd></button>
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
</style>
