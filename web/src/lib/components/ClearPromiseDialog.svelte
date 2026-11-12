<!-- Asks before a done story's own promise is cleared (kb/ADR-0025 R2, R8): it cannot be set again, and the story leaves
     the promise list of the sprint its date falls in, a closed sprint's included. The parent owns the write and passes
     its failure back as `error`. Focus starts on Cancel. -->
<script lang="ts">
	import Dialog from '#lib/ui/Dialog.svelte';

	let {
		item,
		busy = false,
		error = null,
		onconfirm,
		oncancel
	}: {
		/** The done story whose promise would be cleared, named in the title. */
		item: string;
		busy?: boolean;
		error?: string | null;
		onconfirm: () => void;
		oncancel: () => void;
	} = $props();
</script>

<Dialog title="Clear the promise on {item}?" onclose={oncancel} closable={!busy}>
	<div class="clear">
		<p>
			{item} is done, so its promise cannot be set again once cleared, and the story leaves the promise list of the sprint
			its date falls in.
		</p>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div class="actions">
			<button type="button" onclick={oncancel} disabled={busy}>Cancel</button>
			<button type="button" class="primary" onclick={onconfirm} disabled={busy}>Clear promise</button>
		</div>
	</div>
</Dialog>

<style>
	.clear {
		display: grid;
		gap: 12px;
		max-width: min(28rem, 80vw);
	}
	.clear p {
		margin: 0;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}
	.error {
		color: var(--danger);
	}
</style>
