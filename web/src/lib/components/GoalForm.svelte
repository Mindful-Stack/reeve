<!-- web/src/lib/components/GoalForm.svelte: adds a goal to the draft sprint (`POST /goals`; the first goal creates the
     first draft), or, given `goal`, edits that goal (`PATCH /goals/{id}` at its version). Mount a new form under
     `{#key}` of the draft so a half-typed goal never moves to another sprint. -->
<script lang="ts">
	import { api, Conflict, unwrap } from '#lib/api/client.ts';
	import type { CreatedGoal, Goal } from '#lib/api/types.ts';
	import { errorMessage } from '#lib/format.ts';
	import { CreateCycle } from '#lib/idempotency.ts';

	let {
		endsOn = null,
		goal,
		oncreated,
		onsaved,
		onconflict,
		oncancel
	}: {
		/** The draft's last day, for the outcome's shape; null before the first draft exists. */
		endsOn?: string | null;
		/** The goal to edit; without it the form adds one. */
		goal?: Goal;
		/** Add: the goal and the sprint's warnings after the add. */
		oncreated?: (created: CreatedGoal) => Promise<void> | void;
		/** Edit: the goal as saved. */
		onsaved?: (saved: Goal) => Promise<void> | void;
		/** Edit: the goal changed since it was shown; the page refetches and says so, and the form stays open with what was
		 *  typed, so Save again writes it over the current version. */
		onconflict?: () => Promise<void> | void;
		oncancel?: () => void;
	} = $props();

	// The fields start from the goal being edited, once: a reload under an open form leaves the typing alone.
	// svelte-ignore state_referenced_locally
	const start = goal;
	let outcome = $state(start?.outcome ?? '');
	let check = $state(start?.check ?? '');
	let owner = $state(start?.owner ?? '');
	let notes = $state(start?.notes ?? '');
	let error = $state<string | null>(null);
	let busy = $state(false);
	/** A new key after each success, the same key for a retry of the same text, one request per double submit. */
	const cycle = new CreateCycle();

	async function add() {
		const done = oncreated; // the caller's callback for this form, not one passed since to a form that replaced it
		const body = {
			outcome: outcome.trim(),
			check: check.trim(),
			owner: owner.trim() || null, // the server defaults the owner to the caller
			notes: notes.trim() || null
		};
		const out = await cycle.send(JSON.stringify(body), (idempotency_key) =>
			unwrap(api.POST('/api/v1/goals', { body: { ...body, idempotency_key } }))
		);
		if (out.kind === 'sent') {
			outcome = check = owner = notes = '';
			await done?.(out.value);
		} else if (out.kind === 'failed') error = errorMessage(out.error); // the draft and its key stay
	}

	async function save(g: Goal) {
		try {
			const saved = await unwrap(
				api.PATCH('/api/v1/goals/{id}', {
					params: { path: { id: g.id } },
					body: {
						version: g.version,
						outcome: outcome.trim(),
						check: check.trim(),
						owner: owner.trim() || null, // empty leaves the owner as it is
						notes: notes.trim() // empty clears the notes
					}
				})
			);
			await onsaved?.(saved);
		} catch (e) {
			if (e instanceof Conflict) await onconflict?.();
			else error = errorMessage(e);
		}
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (busy || cycle.inFlight || !outcome.trim() || !check.trim()) return;
		busy = true;
		error = null;
		try {
			// The goal as the page shows it now: after a conflict and refetch, its current version.
			if (start && goal) await save(goal);
			else await add();
		} finally {
			busy = false;
		}
	}
</script>

<form class="goal-form" aria-label={start ? 'Edit goal' : 'New goal'} onsubmit={submit}>
	<label class="field"
		>Outcome <input name="outcome" bind:value={outcome} required disabled={busy} placeholder="By {endsOn ?? '<ends_on>'}, <who> can <outcome>" /></label
	>
	<label class="field">We'll know when <input name="check" bind:value={check} required disabled={busy} /></label>
	<label class="field">Owner <input name="owner" type="email" bind:value={owner} disabled={busy} placeholder="you" /></label>
	<label class="field">Notes <textarea name="notes" bind:value={notes} rows="2" disabled={busy}></textarea></label>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<div class="row">
		<button type="submit" class="primary" disabled={busy || !outcome.trim() || !check.trim()}>{start ? 'Save' : 'Add goal'}</button>
		{#if oncancel}<button type="button" class="ghost" disabled={busy} onclick={oncancel}>Cancel</button>{/if}
	</div>
</form>

<style>
	.goal-form {
		display: grid;
		gap: 10px;
	}
	.row {
		display: flex;
		gap: 8px;
	}
	textarea {
		font: inherit;
	}
	.error {
		margin: 0;
		color: var(--danger);
	}
</style>
