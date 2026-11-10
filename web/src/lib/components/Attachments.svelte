<!-- web/src/lib/components/Attachments.svelte: the images an item owns, collapsed under "Attachments · n" below its
     description or brief; nothing at all when it has none. Delete is for good, after a confirm (spec: Screenshots on
     items, §3). A delete, here or seen on the event stream, tells every rendered image of it to show "image
     unavailable" (`reeve:attachment-removed`). -->
<script lang="ts">
	import { tick } from 'svelte';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { AttachmentDto } from '#lib/api/types.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { age, errorMessage, short } from '#lib/format.ts';

	let { itemKey }: { itemKey: string } = $props();

	let list = $state<AttachmentDto[]>([]);
	let error = $state<string | null>(null);
	let busy = $state(false);
	let root = $state<HTMLElement | null>(null);

	const removed = (id: string) => window.dispatchEvent(new CustomEvent('reeve:attachment-removed', { detail: { id } }));

	async function load() {
		try {
			const next = await unwrap(api.GET('/api/v1/items/{key}/attachments', { params: { path: { key: itemKey } } }));
			// One deleted elsewhere (seen on the stream): its rendered images change too.
			for (const a of list) if (!next.some((n) => n.id === a.id)) removed(a.id);
			list = next;
			error = null;
		} catch (e) {
			error = errorMessage(e);
		}
	}

	$effect(() => {
		void itemKey;
		void load();
	});
	onLive(load);

	async function remove(a: AttachmentDto, index: number) {
		if (busy) return;
		if (!confirm(`Delete ${a.filename}? It disappears wherever it is shown, and cannot be restored.`)) return;
		busy = true;
		try {
			await unwrap(api.DELETE('/api/v1/attachments/{id}', { params: { path: { id: a.id } } }));
			list = list.filter((x) => x.id !== a.id);
			removed(a.id);
			error = null;
			// The buttons are enabled again before focus moves to one: a disabled button cannot take it.
			busy = false;
			await tick();
			const buttons = root?.querySelectorAll<HTMLButtonElement>('button.delete');
			const next = buttons?.[Math.min(index, buttons.length - 1)];
			(next ?? root?.querySelector<HTMLElement>('summary'))?.focus();
		} catch (e) {
			error = errorMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

{#if list.length}
	<details class="attachments" bind:this={root}>
		<summary>Attachments · {list.length}</summary>
		<ul>
			{#each list as a, i (a.id)}
				<li>
					<a href={a.url} target="_blank" rel="noopener noreferrer" class="thumb"
						><img src={a.url} alt={a.filename} loading="lazy" decoding="async" /></a
					>
					<a href={a.url} target="_blank" rel="noopener noreferrer" class="name">{a.filename}</a>
					<span class="muted"><span title={a.uploaded_by}>{short(a.uploaded_by)}</span> · {age(a.uploaded_at)}</span>
					<button type="button" class="ghost delete" aria-label="Delete attachment {a.filename}" disabled={busy} onclick={() => remove(a, i)}
						>Delete</button
					>
				</li>
			{/each}
		</ul>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
	</details>
{:else if error}
	<p class="error" role="alert">Could not load attachments: {error}</p>
{/if}

<style>
	.attachments {
		margin-top: 0.75rem;
		font-size: 14px;
	}
	summary {
		cursor: pointer;
		color: var(--muted);
	}
	ul {
		display: grid;
		gap: 0.5rem;
		margin: 0.5rem 0 0;
		padding: 0;
		list-style: none;
	}
	li {
		display: grid;
		grid-template-columns: 3rem 1fr auto auto;
		gap: 0.5rem;
		align-items: center;
	}
	.thumb img {
		display: block;
		width: 3rem;
		height: 2.25rem;
		object-fit: cover;
		border-radius: 3px;
		background: var(--lighter-background);
	}
	.name {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.muted {
		color: var(--muted);
	}
	.error {
		margin: 0.25rem 0 0;
		color: var(--danger);
	}
</style>
