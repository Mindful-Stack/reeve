<!-- The reason prompt every "Mark urgent…" shares. The parent owns the write and passes its failure back as `error`. -->
<script lang="ts">
	import { tick } from 'svelte';
	import Dialog from '#lib/ui/Dialog.svelte';

	let {
		item,
		initial = null,
		busy = false,
		error = null,
		onsubmit,
		oncancel
	}: {
		/** The key of the item being marked, named in the title. */
		item: string;
		/** Prefill for a re-mark. */
		initial?: string | null;
		busy?: boolean;
		error?: string | null;
		onsubmit: (reason: string | null) => void;
		oncancel: () => void;
	} = $props();

	// svelte-ignore state_referenced_locally
	let reason = $state(initial ?? '');
	let input = $state<HTMLInputElement | null>(null);
	// A failed write frees the form: the input was disabled while saving, so focus returns to it.
	$effect(() => {
		if (error && !busy) void tick().then(() => input?.focus());
	});

	function submit(e: SubmitEvent) {
		e.preventDefault();
		if (busy) return;
		onsubmit(reason.trim() || null);
	}
</script>

<Dialog title="Mark {item} urgent" onclose={oncancel} closable={!busy}>
	<form class="mark" aria-label="Mark urgent" onsubmit={submit}>
		<label class="field">Reason (optional) <input bind:this={input} bind:value={reason} maxlength="200" disabled={busy} /></label>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div class="actions">
			<button type="button" onclick={oncancel} disabled={busy}>Cancel</button>
			<button type="submit" class="primary" disabled={busy}>Mark urgent</button>
		</div>
	</form>
</Dialog>

<style>
	.mark {
		display: grid;
		gap: 12px;
		min-width: min(24rem, 80vw);
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}
	.error {
		margin: 0;
		color: var(--danger);
	}
</style>
