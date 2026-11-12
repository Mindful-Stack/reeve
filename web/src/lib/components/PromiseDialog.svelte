<!-- The promise prompt every "Promise by…" and "Change promise…" shares (kb/ADR-0025 R2, R6). The parent owns the write,
     mounts this under an {#if}, and passes its failure back as `error`. Every date comes from the server: the sprints'
     planned ends and `today`; this never reads the client's clock. -->
<script lang="ts">
	import { tick } from 'svelte';
	import type { PromiseFact, PromiseShortcut, SetPromise } from '#lib/api/types.ts';
	import { dayLabel } from '#lib/format.ts';
	import Dialog from '#lib/ui/Dialog.svelte';

	let {
		key,
		activeEnds = null,
		draftEnds = null,
		today = null,
		current = null,
		epicNote = null,
		busy = false,
		error = null,
		onsave,
		onclose
	}: {
		/** The story or epic being promised, named in the title. */
		key: string;
		/** The active sprint's planned end, or null. */
		activeEnds?: string | null;
		/** The draft (next) sprint's planned end, or null. */
		draftEnds?: string | null;
		/** The server's UTC date; until it is known there are no shortcuts and no `min`. */
		today?: string | null;
		/** The item's own promise, when changing it. */
		current?: PromiseFact | null;
		/** "Epic ABC-7 is promised 18 Nov; the earlier date counts.", when the epic's date applies. */
		epicNote?: string | null;
		busy?: boolean;
		error?: string | null;
		onsave: (body: SetPromise) => void;
		onclose: () => void;
	} = $props();

	// svelte-ignore state_referenced_locally
	const startOn = current?.on ?? '';
	// svelte-ignore state_referenced_locally
	const startTo = current?.to ?? '';
	let on = $state(startOn);
	let to = $state(startTo);
	/** The shortcut that filled the date, until the date is edited by hand. */
	let shortcut = $state<PromiseShortcut | null>(null);
	let dateInput = $state<HTMLInputElement | null>(null);

	/** A shortcut is offered only when its sprint's end is today or later (the server refuses a past one). */
	const shortcuts = $derived(
		today == null
			? []
			: (
					[
						['this_sprint', 'End of this sprint', activeEnds],
						['next_sprint', 'End of next sprint', draftEnds]
					] as const
				).flatMap(([s, label, ends]) => (ends != null && ends >= today ? [{ s, label: `${label} (${dayLabel(ends)})`, ends }] : []))
	);
	/** A new date may not be before today; an overdue promise's own date may be kept. */
	const min = $derived(today == null ? undefined : startOn && startOn < today ? startOn : today);

	const body = $derived.by<SetPromise>(() => {
		const b: SetPromise = {};
		if (shortcut) b.shortcut = shortcut;
		else if (on !== startOn) b.on = on;
		const t = to.trim();
		if (t !== startTo) b.to = t;
		return b;
	});
	const canSave = $derived(!busy && on !== '' && Object.keys(body).length > 0);

	// A failed write frees the form: the inputs were disabled while saving, so focus returns to the date.
	$effect(() => {
		if (error && !busy) void tick().then(() => dateInput?.focus());
	});

	function pick(s: PromiseShortcut, ends: string) {
		on = ends;
		shortcut = s;
	}

	function submit(e: SubmitEvent) {
		e.preventDefault();
		if (canSave) onsave(body);
	}
</script>

<Dialog title={current ? `Change the promise on ${key}` : `Promise ${key} by…`} {onclose} closable={!busy}>
	<form class="promise" aria-label="Promise" onsubmit={submit}>
		{#if shortcuts.length}
			<div class="shortcuts">
				{#each shortcuts as sc (sc.s)}
					<button type="button" aria-pressed={shortcut === sc.s} disabled={busy} onclick={() => pick(sc.s, sc.ends)}>{sc.label}</button>
				{/each}
			</div>
		{/if}
		<label class="field"
			>Date <input
				bind:this={dateInput}
				type="date"
				bind:value={on}
				{min}
				required
				disabled={busy}
				oninput={() => (shortcut = null)}
			/></label
		>
		<label class="field">To whom (optional) <input name="to" bind:value={to} maxlength="200" disabled={busy} /></label>
		{#if epicNote}<p class="note">{epicNote}</p>{/if}
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div class="actions">
			<button type="button" onclick={onclose} disabled={busy}>Cancel</button>
			<button type="submit" class="primary" disabled={!canSave}>Promise</button>
		</div>
	</form>
</Dialog>

<style>
	.promise {
		display: grid;
		gap: 12px;
		min-width: min(24rem, 80vw);
	}
	.shortcuts {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.note {
		margin: 0;
		color: var(--muted);
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
