<!-- web/src/lib/components/ImageUploadsRow.svelte: under an attachable Markdown field, outside its label: the Add image
     button, the hint, the uploads of this edit and a live status (spec: Screenshots on items, §3). -->
<script lang="ts">
	import { TYPES, type ImageUploads } from '#lib/attach.svelte.ts';

	let { uploads, field, disabled = false }: { uploads: ImageUploads; field?: HTMLTextAreaElement | null; disabled?: boolean } = $props();
	let input = $state<HTMLInputElement | null>(null);

	let addButton = $state<HTMLButtonElement | null>(null);

	/** The row's button leaves the page with it: focus goes back to the field, or to Add image. */
	function remove(p: (typeof uploads.pending)[number]) {
		uploads.remove(p);
		(field && !field.disabled ? field : addButton)?.focus();
	}

	function chosen(e: Event) {
		const el = e.currentTarget as HTMLInputElement;
		const files = [...(el.files ?? [])];
		el.value = '';
		if (files.length) uploads.add(files, field);
		field?.focus();
	}
</script>

<div class="uploads">
	<button type="button" class="ghost" bind:this={addButton} onclick={() => input?.click()} {disabled}>Add image</button>
	<input bind:this={input} type="file" accept={TYPES.join(',')} multiple hidden onchange={chosen} />
	<span class="hint">Paste, drop or add an image</span>
	{#if uploads.pending.length}
		<ul>
			{#each uploads.pending as p (p.token)}
				<li class={p.state}>
					{#if p.url}<img src={p.url} alt="" class="thumb" />{/if}
					<span class="name">{p.name}</span>
					<span class="state">
						{#if p.state === 'uploading'}Uploading…{:else if p.state === 'added'}added{:else}{p.error}{/if}
					</span>
					<button type="button" class="ghost" aria-label="Remove {p.name}" onclick={() => remove(p)}>Remove</button>
				</li>
			{/each}
		</ul>
	{/if}
	<p class="visually-hidden" aria-live="polite">{uploads.status}</p>
</div>

<style>
	.uploads {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 0.25rem 0.5rem;
		margin-top: 0.25rem;
		font-size: 13px;
	}
	.hint {
		color: var(--muted);
	}
	ul {
		flex-basis: 100%;
		display: grid;
		gap: 0.125rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	li {
		display: flex;
		gap: 0.5rem;
		align-items: center;
	}
	.thumb {
		flex: none;
		width: 2rem;
		height: 1.5rem;
		object-fit: cover;
		border-radius: 2px;
	}
	.name {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.state {
		flex: none;
		color: var(--muted);
	}
	li button {
		flex: none;
	}
	.failed .state {
		color: var(--danger);
	}
</style>
