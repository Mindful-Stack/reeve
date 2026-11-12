<!-- web/src/lib/components/CloseSprint.svelte: the close step (Plan Tuesday on one screen). Every goal with its
     check, latest confidence and completed count asks a verdict and a note; Close is enabled once each has a verdict; a
     closed sprint offers "Open next sprint", which opens the draft. -->
<script lang="ts">
	import { api, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import type { Sprint, SprintView, SprintWarning, Verdict } from '#lib/api/types.ts';
	import { fullLabel } from '#lib/board.ts';
	import { errorMessage } from '#lib/format.ts';
	import { confidenceLabel, verdictLabel, warningText } from '#lib/sprint.ts';

	let {
		view,
		draft,
		leaveOut,
		onchanged,
		ondone
	}: {
		/** The active sprint as the step was opened on: its goals, version and counts. */
		view: SprintView;
		/** The draft to open once this sprint is closed (the page's current one), or null. */
		draft: Sprint | null;
		/** Draft goals unticked on the page: they stay in the next draft (kb/ADR-0026). Chosen before Close…, since the
		 *  dialog is modal. */
		leaveOut: number[];
		/** A write landed or the sprint changed: the page reloads. */
		onchanged: () => Promise<void> | void;
		/** The step is over (the next sprint opened). */
		ondone: () => void;
	} = $props();

	const VERDICTS: Verdict[] = ['met', 'partly_met', 'missed'];
	// The step's own copy of the sprint it asks about: a reload after the close must not empty the list under it. It
	// is refetched only when a refusal names a goal the step does not list (one added since the step opened).
	// svelte-ignore state_referenced_locally
	let current = $state(view);
	const goals = $derived(current.sprint.goals);
	const completed = $derived(new Map(current.counts.goals.map((g) => [g.goal_id, g.completed])));

	// svelte-ignore state_referenced_locally
	let verdicts = $state<Record<number, Verdict | null>>(Object.fromEntries(view.sprint.goals.map((g) => [g.id, null])));
	// svelte-ignore state_referenced_locally
	let notes = $state<Record<number, string>>(Object.fromEntries(view.sprint.goals.map((g) => [g.id, ''])));
	/** The goals a `verdict_missing` refusal named. */
	let missing = $state<ReadonlySet<number>>(new Set());
	let error = $state<string | null>(null);
	let busy = $state(false);
	let closed = $state(false);
	/** The next sprint opened with advice: the step stays to show it until Done. */
	let warnings = $state<SprintWarning[] | null>(null);

	const goingIn = $derived((draft?.goals.length ?? 0) - leaveOut.length);
	const goalsWord = (n: number) => `${n} ${n === 1 ? 'goal' : 'goals'}`;

	const complete = $derived(goals.every((g) => (verdicts[g.id] ?? null) !== null));

	/** The sprint as it is now, keeping every verdict and note already chosen; a new goal starts without one. */
	async function refetch() {
		const fresh = await unwrap(api.GET('/api/v1/sprints/{id}', { params: { path: { id: current.sprint.id } } }));
		for (const g of fresh.sprint.goals) {
			verdicts[g.id] ??= null;
			notes[g.id] ??= '';
		}
		current = fresh;
	}

	function explain(e: unknown): string {
		return e instanceof Refused ? e.reasons.map(fullLabel).join('; ') : errorMessage(e);
	}

	async function close() {
		if (busy || !complete) return;
		busy = true;
		error = null;
		try {
			await unwrap(
				api.POST('/api/v1/sprints/{id}/close', {
					params: { path: { id: current.sprint.id } },
					body: {
						version: current.sprint.version,
						verdicts: goals.map((g) => ({ goal_id: g.id, verdict: verdicts[g.id]!, note: notes[g.id].trim() || null }))
					}
				})
			);
			closed = true;
			missing = new Set();
			await onchanged();
		} catch (e) {
			const named = e instanceof Refused ? e.reasons.flatMap((r) => (r.reason === 'verdict_missing' ? r.goals : [])) : [];
			missing = new Set(named);
			error = explain(e);
			if (named.some((id) => !goals.some((g) => g.id === id))) {
				try {
					await refetch();
				} catch (f) {
					error = `${error}; could not load the new goals: ${errorMessage(f)}`;
				}
			}
			if (e instanceof Conflict) await onchanged();
		} finally {
			busy = false;
		}
	}

	async function openNext() {
		if (busy || !draft) return;
		busy = true;
		error = null;
		try {
			const opened = await unwrap(
				api.POST('/api/v1/sprints/{id}/open', { params: { path: { id: draft.id } }, body: { version: draft.version, leave_out: leaveOut } })
			);
			await onchanged();
			if (opened.warnings.length === 0) ondone();
			else warnings = opened.warnings;
		} catch (e) {
			error = explain(e);
			await onchanged();
		} finally {
			busy = false;
		}
	}
</script>

<div class="close-step" data-testid="close-sprint">
	{#if !closed}
		<ol class="goals">
			{#each goals as g (g.id)}
				<li data-testid="close-goal-{g.id}" class:missing={missing.has(g.id)}>
					<p class="outcome">{g.outcome}</p>
					<p class="muted">We'll know when: {g.check}</p>
					<p class="muted">
						Confidence: {g.confidence ? confidenceLabel(g.confidence) : 'not recorded'} · {completed.get(g.id) ?? 0} completed
					</p>
					<fieldset role="radiogroup" disabled={busy} aria-invalid={missing.has(g.id) ? 'true' : undefined}>
						<legend class="visually-hidden">Verdict for {g.outcome}</legend>
						{#each VERDICTS as v (v)}
							<label
								><input type="radio" name="verdict-{g.id}" value={v} bind:group={verdicts[g.id]} />
								{verdictLabel(v)}</label
							>
						{/each}
					</fieldset>
					<label class="field">Note <input bind:value={notes[g.id]} disabled={busy} /></label>
					{#if missing.has(g.id)}<p class="error">This goal needs a verdict.</p>{/if}
				</li>
			{/each}
		</ol>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<button type="button" class="primary" disabled={busy || !complete} onclick={close}>Close sprint</button>
	{:else if warnings}
		<p role="status">The new sprint is open.</p>
		<p class="warn">Warning: {warnings.map((w) => warningText(w)).join('; ')}</p>
		<button type="button" onclick={ondone}>Done</button>
	{:else}
		<p role="status">The sprint is closed.</p>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		{#if draft}
			{#if leaveOut.length}<p class="muted">{goalsWord(leaveOut.length)} {leaveOut.length === 1 ? 'stays' : 'stay'} in Next sprint.</p>{/if}
			<button type="button" class="primary" disabled={busy} onclick={openNext}>Open next sprint with {goalsWord(goingIn)}</button>
		{:else}
			<p class="muted">There is no draft to open: add a goal to start one.</p>
		{/if}
	{/if}
</div>

<style>
	.close-step {
		display: grid;
		gap: 12px;
	}
	.goals {
		display: grid;
		gap: 14px;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.goals li {
		display: grid;
		gap: 4px;
		padding-left: 8px;
		border-left: 2px solid transparent;
	}
	.goals li.missing {
		border-left-color: var(--danger);
	}
	p {
		margin: 0;
	}
	.outcome {
		font-weight: 600;
	}
	fieldset {
		display: flex;
		flex-wrap: wrap;
		gap: 12px;
		margin: 0;
		padding: 0;
		border: 0;
	}
	button {
		justify-self: start;
	}
	.error {
		color: var(--danger);
	}
	.warn {
		color: var(--warn);
	}
</style>
