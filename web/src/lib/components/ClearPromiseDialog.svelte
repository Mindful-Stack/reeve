<!-- Asks before a promise is cleared that cannot be put back as it was (kb/ADR-0025 R2, R3, R8). A done story's own
     promise cannot be set again, and the story leaves the promise list of the sprint its date falls in, unless it still
     inherits its epic's promise (then it stays listed under the epic's date). An epic's promise, set again later, gets
     a new time, so its stories done by then no longer inherit it and their kept and late rows leave the closed sprints'
     lists. The parent owns the write and passes its failure back as `error`. Focus starts on Cancel, and returns there
     after a failed clear. -->
<script lang="ts">
	import { tick } from 'svelte';
	import Dialog from '#lib/ui/Dialog.svelte';

	let {
		item,
		kind = 'story',
		epic = null,
		busy = false,
		error = null,
		onconfirm,
		oncancel
	}: {
		/** The done story or the epic whose promise would be cleared, named in the title. */
		item: string;
		kind?: 'story' | 'epic';
		/** For a story: the epic whose promise it also inherits (`story.promise.epic`), which the clear leaves. */
		epic?: string | null;
		busy?: boolean;
		error?: string | null;
		onconfirm: () => void;
		oncancel: () => void;
	} = $props();

	let cancel = $state<HTMLButtonElement>();
	// A failed clear frees the buttons: they were disabled while it ran, so focus returns to Cancel.
	$effect(() => {
		if (error && !busy) void tick().then(() => cancel?.focus());
	});
</script>

<Dialog title="Clear the promise on {item}?" onclose={oncancel} closable={!busy}>
	<div class="clear">
		<p data-testid="clear-promise-text">
			{#if kind === 'epic'}
				Its stories lose this date. If {item} is promised again later, its stories done by then do not get the new
				promise, so they leave the promise lists of the sprints already closed.
			{:else if epic}
				{item} is done, so its own promise cannot be set again once cleared. It stays listed under epic {epic}'s
				date.
			{:else}
				{item} is done, so its own promise cannot be set again once cleared, and the story leaves the promise list of
				the sprint its date falls in.
			{/if}
		</p>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div class="actions">
			<button type="button" bind:this={cancel} onclick={oncancel} disabled={busy}>Cancel</button>
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
