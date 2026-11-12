<!-- The promise prompt every "Promise by…" and "Change promise…" shares (kb/ADR-0025 R2, R6). The parent owns the write,
     mounts this under an {#if}, and passes its failure back as `error`. Every date comes from the server: the sprints'
     planned ends and `today`; this never reads the client's clock. -->
<script lang="ts">
	import { tick } from 'svelte';
	import type { PromiseFact, PromiseShortcut, SetPromise } from '#lib/api/types.ts';
	import { dayLabel } from '#lib/format.ts';
	import { epicNote } from '#lib/promise.ts';
	import Dialog from '#lib/ui/Dialog.svelte';

	let {
		key,
		activeEnds = null,
		draftEnds = null,
		today = null,
		sprintsFailed = false,
		current = null,
		epic = null,
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
		/** The sprints could not be read: there will be no shortcuts, and the dialog says so instead of waiting. */
		sprintsFailed?: boolean;
		/** The item's own promise, when changing it. */
		current?: PromiseFact | null;
		/** The epic promise the story inherits, whichever date is earlier: the note says which one counts. */
		epic?: { key: string; on: string } | null;
		busy?: boolean;
		error?: string | null;
		onsave: (body: SetPromise) => void;
		onclose: () => void;
	} = $props();

	const PAST = 'the date is past: promise today or later';
	/** The server's limit, counted as it counts: in Unicode code points of the trimmed text, not UTF-16 units (which
	 *  `maxlength` counts). A UX affordance; the server re-checks. */
	const MAX_RECIPIENT = 200;
	const TOO_LONG = `the recipient is longer than ${MAX_RECIPIENT} characters`;
	const id = $props.id();

	// svelte-ignore state_referenced_locally
	const startOn = current?.on ?? '';
	// svelte-ignore state_referenced_locally
	const startTo = current?.to ?? '';
	let on = $state(startOn);
	let to = $state(startTo);
	/** The shortcut that filled the date, until the date is edited by hand. */
	let shortcut = $state<PromiseShortcut | null>(null);
	let dateInput = $state<HTMLInputElement | null>(null);
	let toInput = $state<HTMLInputElement | null>(null);

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

	/** The end now offered for the picked shortcut. The host refetches the sprints while the dialog is open, so it can
	 *  move (an edited end) or go (a closed sprint) after the pick. */
	const offered = $derived(shortcut == null ? null : (shortcuts.find((x) => x.s === shortcut)?.ends ?? null));
	/** The shortcut is sent only while its end is still the date in the field; otherwise the date the person sees is. */
	const sendShortcut = $derived(shortcut != null && offered === on);

	const body = $derived.by<SetPromise>(() => {
		const b: SetPromise = {};
		if (sendShortcut && shortcut) b.shortcut = shortcut;
		else if (on !== startOn) b.on = on;
		const t = to.trim();
		if (t !== startTo) b.to = t;
		return b;
	});
	const canSave = $derived(!busy && on !== '' && Object.keys(body).length > 0);
	/** A changed date before today: `min` allows it only to keep an overdue promise's own date. */
	const past = $derived(today != null && on !== startOn && on !== '' && on < today);
	$effect(() => {
		dateInput?.setCustomValidity(past ? PAST : '');
	});
	const tooLong = $derived([...to.trim()].length > MAX_RECIPIENT);
	$effect(() => {
		toInput?.setCustomValidity(tooLong ? TOO_LONG : '');
	});
	const note = $derived(epic ? epicNote(epic, on) : null);

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
		if (past) {
			dateInput?.reportValidity();
			return;
		}
		if (tooLong) {
			toInput?.reportValidity();
			return;
		}
		if (canSave) onsave(body);
	}
</script>

<!-- The title follows the frozen base: a reload while the dialog is open must not retitle it. -->
<Dialog title={startOn !== '' ? `Change the promise on ${key}` : `Promise ${key} by…`} {onclose} closable={!busy}>
	<form class="promise" aria-label="Promise" onsubmit={submit}>
		<label class="field"
			>Date <input
				bind:this={dateInput}
				type="date"
				bind:value={on}
				{min}
				required
				disabled={busy}
				aria-describedby={note ? `${id}-note` : undefined}
				oninput={() => (shortcut = null)}
			/></label
		>
		<!-- Below the date, in a row reserved while the sprints are unknown (`today` null): shortcuts that arrive with a late
		     read (the epic page fetches them on open) never move the field being used. Once known with none to offer, or
		     once the read has failed, the row collapses. -->
		{#if sprintsFailed}
			<p class="note" data-testid="promise-shortcuts-unavailable">Sprint shortcuts are unavailable.</p>
		{:else if today == null || shortcuts.length > 0}
			<div class="shortcuts" data-testid="promise-shortcuts">
				{#each shortcuts as sc (sc.s)}
					<button type="button" aria-pressed={shortcut === sc.s && sc.ends === on} disabled={busy} onclick={() => pick(sc.s, sc.ends)}
						>{sc.label}</button
					>
				{/each}
			</div>
		{/if}
		<label class="field">To whom (optional) <input name="to" bind:this={toInput} bind:value={to} disabled={busy} /></label>
		<!-- Polite: the note changes with the date being chosen, and says which date counts. -->
		{#if note}<p class="note" id="{id}-note" aria-live="polite">{note}</p>{/if}
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<div class="actions">
			<button type="button" onclick={onclose} disabled={busy}>Cancel</button>
			<button type="submit" class="primary" disabled={!canSave}>{startOn !== '' ? 'Save' : 'Promise'}</button>
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
		min-height: 2rem;
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
