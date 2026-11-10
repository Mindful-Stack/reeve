<!-- web/src/lib/components/ImageUploadsRow.svelte: under an attachable Markdown field, outside its label: the Add image
     button, the hint, the uploads of this edit and a live status (spec: Screenshots on items, §3). -->
<script lang="ts">
	import { TYPES, type ImageUploads } from '#lib/attach.svelte.ts';

	let { uploads, field, disabled = false }: { uploads: ImageUploads; field?: HTMLTextAreaElement | null; disabled?: boolean } = $props();
	let input = $state<HTMLInputElement | null>(null);

	function chosen(e: Event) {
		const el = e.currentTarget as HTMLInputElement;
		const files = [...(el.files ?? [])];
		el.value = '';
		if (files.length) uploads.add(files, field);
		field?.focus();
	}
</script>

<div class="uploads">
	<button type="button" class="ghost" onclick={() => input?.click()} {disabled}>Add image</button>
	<input bind:this={input} type="file" accept={TYPES.join(',')} multiple hidden onchange={chosen} />
	<span class="hint">Paste, drop or add an image</span>
	{#if uploads.pending.length}
		<ul>
			{#each uploads.pending as p (p.token)}
				<li class={p.state}>
					<span class="name">{p.name}</span>
					<span class="state">
						{#if p.state === 'uploading'}Uploading…{:else if p.state === 'added'}added{:else}{p.error}{/if}
					</span>
					<button type="button" class="ghost" aria-label="Remove {p.name}" onclick={() => uploads.remove(p)}>Remove</button>
				</li>
			{/each}
		</ul>
	{/if}
	<p class="sr-only" aria-live="polite">{uploads.status}</p>
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
	.name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.state {
		color: var(--muted);
	}
	.failed .state {
		color: var(--danger);
	}
	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
		white-space: nowrap;
		margin: 0;
	}
</style>
