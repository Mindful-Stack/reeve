<!-- web/src/lib/components/GoalPicker.svelte: the header's goal picker. Its dialog lists the next and the active sprint's
     goals (never a closed sprint's, kb/ADR-0014 R6) and adds goals to the draft; picking a goal starts linking mode,
     in which initiatives and epics show a GoalLinkButton. Goals are edited on /sprint. -->
<script lang="ts">
	import { tick } from 'svelte';
	import { onLive } from '#lib/events.svelte.ts';
	import { linkingGoal, loadSprints, pickable, picker, startLinking, stopLinking } from '#lib/goalPicker.svelte.ts';
	import Dialog from '#lib/ui/Dialog.svelte';
	import GoalForm from './GoalForm.svelte';

	let open = $state(false);
	let adding = $state(false);
	// Bumped whenever a form opens or goes away, so a slow add answering for a dismissed form leaves a newer one alone.
	let form = 0;
	let created = $state(() => added(form));
	let pillBtn = $state<HTMLButtonElement | null>(null);
	let goalsBtn = $state<HTMLButtonElement | null>(null);
	let addBtn = $state<HTMLButtonElement | null>(null);

	const goal = $derived(linkingGoal());
	const groups = $derived(pickable(picker.sprints));

	// Someone else's goal or link reaches an open dialog or the pill (#lib/events.svelte.ts).
	onLive(() => {
		if (open || picker.goalId !== null) void loadSprints();
	});

	function show() {
		open = true;
		void loadSprints();
	}

	function startAdding() {
		const run = ++form;
		created = () => added(run);
		adding = true;
	}

	function stopAdding() {
		form++;
		adding = false;
	}

	function close() {
		open = false;
		stopAdding();
		picker.error = null;
	}

	function pick(id: number) {
		startLinking(id);
		close();
		void tick().then(() => pillBtn?.focus());
	}

	function stop() {
		stopLinking();
		void tick().then(() => goalsBtn?.focus());
	}

	async function added(run: number) {
		await loadSprints();
		if (run !== form) return;
		stopAdding();
		await tick();
		addBtn?.focus();
	}
</script>

<div class="goal-picker">
	{#if goal}
		<span class="pill">
			<button bind:this={pillBtn} type="button" class="ghost" onclick={show}><span class="lead">{'Linking to: '}</span><span class="outcome">{goal.outcome}</span></button
			><button type="button" class="ghost icon" aria-label="Stop linking" onclick={stop}>✕</button>
		</span>
	{:else}
		<button bind:this={goalsBtn} type="button" class="ghost" onclick={show}>Goals ▾</button>
	{/if}
	{#if picker.error && !open}<p class="error" role="alert">{picker.error}</p>{/if}
</div>

{#if open}
	<Dialog title="Sprint goals" onclose={close}>
		{#if picker.error}<p class="error" role="alert">{picker.error}</p>{/if}
		{#if picker.sprints === null && !picker.error}
			<p class="muted">Loading…</p>
		{:else if picker.sprints}
			{#each groups as g (g.sprint.id)}
				<section>
					<h3>{g.label}</h3>
					{#if g.sprint.goals.length === 0}<p class="muted">No goals yet</p>{/if}
					<ul>
						{#each g.sprint.goals as item (item.id)}
							<li>
								<button type="button" class="goal" aria-pressed={picker.goalId === item.id} onclick={() => pick(item.id)}
									><span>{item.outcome}</span> <span class="count" title="Linked items">{item.links.length}</span></button
								>
							</li>
						{/each}
					</ul>
				</section>
			{:else}
				<p class="muted">No sprint planned yet</p>
			{/each}
			{#if adding}
				<!-- Keyed on the draft: a half-typed goal never moves to another sprint (GoalForm). -->
				{#key picker.sprints.draft?.id ?? 0}
					<GoalForm endsOn={picker.sprints.draft?.ends_on ?? null} oncreated={created} oncancel={stopAdding} />
				{/key}
			{:else}
				<button bind:this={addBtn} type="button" class="ghost" onclick={startAdding}>+ Add goal</button>
			{/if}
		{/if}
		<p><a href="/sprint" onclick={close}>Open sprint page</a></p>
	</Dialog>
{/if}

<style>
	.goal-picker {
		position: relative;
		min-width: 0;
	}
	.pill {
		display: inline-flex;
		align-items: center;
		border: 1px solid var(--accent);
		border-radius: 999px;
	}
	.outcome {
		display: inline-block;
		max-width: min(28ch, 30vw);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		vertical-align: bottom;
	}
	@media (max-width: 720px) {
		.lead {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip-path: inset(50%);
			white-space: nowrap;
		}
	}
	h3 {
		margin: 12px 0 4px;
		font-size: 0.85em;
	}
	ul {
		display: grid;
		gap: 4px;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.goal {
		display: flex;
		justify-content: space-between;
		gap: 8px;
		width: 100%;
		height: auto;
		min-height: 32px;
		padding-block: 6px;
		text-align: left;
	}
	.goal[aria-pressed='true'] {
		border-color: var(--accent);
	}
	.count {
		color: var(--muted);
	}
	.error {
		margin: 0;
		color: var(--danger);
	}
	.goal-picker .error {
		position: absolute;
		right: 0;
		top: calc(100% + 6px);
		max-width: 40ch;
		background: var(--raised);
		padding: 4px 8px;
		border: 1px solid var(--line);
		border-radius: 4px;
		z-index: 1;
	}
</style>
