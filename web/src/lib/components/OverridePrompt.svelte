<script lang="ts">
	import type { Reason } from '#lib/api/types.ts';
	import { fullLabel } from '#lib/board.ts';
	import Dialog from '#lib/ui/Dialog.svelte';

	// Shown when a move is refused for readiness reasons. Submitting re-posts the move with `override_reason`; the card stays
	// where it was until that succeeds.
	let {
		storyKey,
		target,
		reasons,
		busy = false,
		onconfirm,
		oncancel
	}: {
		storyKey: string;
		/** The column label the story is being moved to. */
		target: string;
		reasons: Reason[];
		busy?: boolean;
		onconfirm: (reason: string) => void;
		oncancel: () => void;
	} = $props();

	let why = $state('');

	function submit(e: SubmitEvent) {
		e.preventDefault();
		if (why.trim()) onconfirm(why.trim());
	}
</script>

<Dialog title="{storyKey} is not ready for {target}" onclose={oncancel} closable={!busy}>
	<form onsubmit={submit}>
		<ul>
			{#each reasons as r, i (i)}
				<li>{fullLabel(r)}</li>
			{/each}
		</ul>
		<label class="field">
			Reason
			<textarea bind:value={why} rows="3" required placeholder="Why move it anyway?"></textarea>
		</label>
		<div class="actions">
			<button type="button" onclick={oncancel} disabled={busy}>Cancel</button>
			<button type="submit" class="primary" disabled={busy || !why.trim()}>Move anyway</button>
		</div>
	</form>
</Dialog>

<style>
	ul {
		margin: 0 0 12px;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 12px;
	}
</style>
