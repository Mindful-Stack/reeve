<!-- web/src/routes/sprint/+page.svelte: the sprint page, read on a shared screen when the team plans. This sprint (the
     active one) and the next (the draft) side by side, every goal one line that opens in place (`GoalRow`); "+ goal"
     under each; the first horizon bucket's initiatives with the goals that reach them (`GET /sprints/planning`, the
     server's comparison, display only: kb/ADR-0026); then the demo list and the history, folded. The kb/ADR-0014 R2
     rule line stands above the counts. -->
<script lang="ts">
	import { tick } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { api, Refused, unwrap } from '#lib/api/client.ts';
	import type { Confidence, Goal, Sprints, SprintPlanning, SprintView, SprintWarning } from '#lib/api/types.ts';
	import { fullLabel } from '#lib/board.ts';
	import CloseSprint from '#lib/components/CloseSprint.svelte';
	import GoalForm from '#lib/components/GoalForm.svelte';
	import GoalRow from '#lib/components/GoalRow.svelte';
	import SprintHistoryEntry from '#lib/components/SprintHistoryEntry.svelte';
	import { onLive } from '#lib/events.svelte.ts';
	import { errorMessage, month, plural } from '#lib/format.ts';
	import { confidenceLabel, RULE, upkeepText, warningText } from '#lib/sprint.ts';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import { itemRoute } from '#lib/urgency.ts';

	let sprints = $state<Sprints | null>(null);
	/** The server's planning answer: each active and draft goal's gates and initiatives, and the first bucket's
	 *  initiatives. A goal without an entry (the reads raced an opening) shows no tag or gates until the next
	 *  refetch, which the opening's live event triggers. */
	let plan = $state.raw<SprintPlanning | null>(null);
	const gates = $derived(new Map((plan?.goals ?? []).map((x) => [x.goal_id, x])));
	const firstBucket = $derived(plan?.first_bucket ?? null);
	let loadError = $state<string | null>(null);
	/** A refetch failed after the page had loaded: what is shown stays; the banner offers a retry. */
	let staleNotice = $state<string | null>(null);
	/** A write the server refused or failed, in words. */
	let notice = $state<string | null>(null);
	let busy = $state(false);

	const active = $derived(sprints?.active ?? null);
	const draft = $derived(sprints?.draft ?? null);

	// --- Open goals ----------------------------------------------------------------------------------------------------
	/** The goals whose detail shows, by id: page state, so a live refetch keeps them. */
	const opened = new SvelteSet<number>();
	const toggle = (id: number) => (opened.has(id) ? opened.delete(id) : opened.add(id));

	let latest = 0;
	/** Bumped on every reload, for the history entries. */
	let refresh = $state(0);
	async function reload() {
		const seq = ++latest;
		try {
			// Two reads together: they land or fail together, so the page never shows a partial state.
			const [s, p] = await Promise.all([unwrap(api.GET('/api/v1/sprints')), unwrap(api.GET('/api/v1/sprints/planning'))]);
			if (seq !== latest) return;
			// A new draft (an opening happened): nothing is unticked in it.
			if (s.draft?.id !== sprints?.draft?.id) {
				leftOut.clear();
				// The "+ goal" form is for the draft it was opened on: a new draft never receives its half-typed goal.
				goalFor = null;
			}
			sprints = s;
			plan = p;
			// A goal that left both sprints is no longer open.
			const here = new Set([...(s.active?.sprint.goals ?? []), ...(s.draft?.goals ?? [])].map((g) => g.id));
			for (const id of opened) if (!here.has(id)) opened.delete(id);
			loadError = null;
			staleNotice = null;
		} catch (e) {
			if (seq !== latest) return;
			const m = errorMessage(e);
			if (sprints) staleNotice = `Could not reload the sprint plan: ${m}`;
			else loadError = m;
		}
		// An opened history entry is derived on read too: it refetches with the page.
		refresh++;
	}

	$effect(() => {
		reload();
	});
	// Any activity entry, and every (re)connect, refetches (#lib/events.svelte.ts).
	onLive(reload);

	function explain(e: unknown): string {
		return e instanceof Refused ? e.reasons.map(fullLabel).join('; ') : errorMessage(e);
	}

	/** One write: the page reloads after it, and a refusal or failure shows as the notice. */
	async function act(write: () => Promise<unknown>): Promise<boolean> {
		if (busy) return false;
		busy = true;
		notice = null;
		try {
			await write();
			return true;
		} catch (e) {
			notice = explain(e);
			return false;
		} finally {
			await reload();
			busy = false;
		}
	}

	// --- Confidence (Check Tuesday) ---------------------------------------------------------------------------------
	const CONFIDENCES: Confidence[] = ['on_track', 'at_risk', 'off_track'];
	/** The goal whose confidence form is open. A suggestion pre-fills it; nothing is posted until Record. */
	let confidenceFor = $state<Goal | null>(null);
	let choice = $state<Confidence | null>(null);
	let confidenceNote = $state('');

	function openConfidence(g: Goal) {
		confidenceFor = g;
		choice = g.suggested_confidence ?? null;
		confidenceNote = g.suggested_note ?? '';
	}

	async function recordConfidence(e: SubmitEvent) {
		e.preventDefault();
		const g = confidenceFor;
		if (!g || !choice) return;
		const confidence = choice;
		const ok = await act(() =>
			unwrap(
				api.POST('/api/v1/goals/{id}/confidence', {
					params: { path: { id: g.id } },
					body: { confidence, note: confidenceNote.trim() || null }
				})
			)
		);
		if (ok) confidenceFor = null;
	}

	// --- The draft (Monday prep) -----------------------------------------------------------------------------------
	/** Draft goals the person unticked: they stay in the next draft at the opening (kb/ADR-0026). */
	const leftOut = new SvelteSet<number>();
	/** What an opening sends: unticked ids still in the draft, so a removed goal is never sent and a new one is ticked. */
	const leaveOut = $derived((draft?.goals ?? []).filter((g) => leftOut.has(g.id)).map((g) => g.id));
	const goingIn = $derived((draft?.goals.length ?? 0) - leaveOut.length);
	/** Advice before the opening, from the goals that go in (the server warns again after it). */
	const draftWarning = $derived<SprintWarning[]>(
		goingIn > 3 ? ['more_than_three_goals'] : draft?.goals.length && goingIn === 0 ? ['no_goals'] : []
	);
	let linkKeys = $state<Record<number, string>>({});

	async function link(e: SubmitEvent, g: Goal) {
		e.preventDefault();
		const key = (linkKeys[g.id] ?? '').trim();
		if (!key) return;
		const ok = await act(() => unwrap(api.POST('/api/v1/goals/{id}/links', { params: { path: { id: g.id } }, body: { items: [key] } })));
		if (ok) linkKeys[g.id] = '';
	}
	const unlink = (g: Goal, key: string) =>
		act(() => unwrap(api.DELETE('/api/v1/goals/{id}/links/{key}', { params: { path: { id: g.id, key } } })));
	const remove = (g: Goal) =>
		act(() => unwrap(api.DELETE('/api/v1/goals/{id}', { params: { path: { id: g.id }, query: { version: g.version } } })));

	/** The goal whose edit form is open (on either panel). */
	let editing = $state<number | null>(null);
	async function edited() {
		editing = null;
		await reload();
	}
	/** The edit form stays open with what was typed; it saves again at the refetched goal's version. */
	async function editConflict() {
		notice = 'someone else changed this goal; reloaded';
		await reload();
	}

	/** Opens the draft when no sprint is active (the first sprint, or after a close outside the close step). */
	let openWarnings = $state<SprintWarning[]>([]);
	async function openDraft() {
		const d = draft;
		if (!d) return;
		await act(async () => {
			const opened = await unwrap(
				api.POST('/api/v1/sprints/{id}/open', { params: { path: { id: d.id } }, body: { version: d.version, leave_out: leaveOut } })
			);
			openWarnings = opened.warnings;
		});
	}

	// --- Close (Plan Tuesday) --------------------------------------------------------------------------------------
	/** The active sprint as the close step was opened on; the step keeps it while the page reloads under it. */
	let closing = $state<SprintView | null>(null);

	// --- "+ goal" ---------------------------------------------------------------------------------------------------
	/** Every goal of both sprints, with which sprint it is in, for the bucket list's chips. */
	const everyGoal = $derived(
		new Map<number, { goal: Goal; where: string }>([
			...(active?.sprint.goals ?? []).map((g) => [g.id, { goal: g, where: 'this sprint' }] as const),
			...(draft?.goals ?? []).map((g) => [g.id, { goal: g, where: 'next sprint' }] as const)
		])
	);
	/** The open "+ goal" form: an initiative's key (the bucket list), `'active'` (this sprint's column), or none. */
	let goalFor = $state<string | null>(null);
	/** The bucket rows, by initiative key, to put focus inside them. */
	const rows: Record<string, HTMLElement | undefined> = {};
	/** The new-goal form under this sprint: its own element, so the query never finds an Edit form's inputs. */
	let activeForm = $state<HTMLElement | undefined>();
	const holder = (key: string) => (key === 'active' ? activeForm : rows[key]);
	/** Opens the form and focuses its outcome input once it is in the DOM. */
	async function openGoal(key: string) {
		goalFor = key;
		await tick();
		holder(key)?.querySelector<HTMLInputElement>('input[name="outcome"]')?.focus();
	}
	/** Cancels the form and returns focus to the "+ goal" button that opened it. */
	async function closeGoal(key: string) {
		goalFor = null;
		await tick();
		holder(key)?.querySelector<HTMLButtonElement>('button.add-goal')?.focus();
	}
	/** After an add: the form closes, the page reloads, and focus goes to the new goal's row. */
	async function added(id: number) {
		goalFor = null;
		await reload();
		await tick();
		document.getElementById(`goal-${id}`)?.focus();
	}
</script>

<!-- A goal's controls, shown in its opened row: its goal links with unlink, the link field, the confidence form or
     the actions, and the edit form. The same for both sprints, except confidence (active) and Remove (draft). -->
{#snippet goalControls(g: Goal, isActive: boolean)}
	{#if g.links.length}
		<ul class="links" aria-label="Linked items">
			{#each g.links as key (key)}
				<li>
					<span class="key">{key}</span>
					<button type="button" class="ghost icon" aria-label="Unlink {key}" disabled={busy} onclick={() => unlink(g, key)}>×</button>
				</li>
			{/each}
		</ul>
	{/if}
	<form class="link" aria-label="Link to {g.outcome}" onsubmit={(e) => link(e, g)}>
		<input name="link" bind:value={linkKeys[g.id]} placeholder="Epic or initiative key" aria-label="Epic or initiative key" disabled={busy} />
		<button type="submit" disabled={busy}>Link</button>
	</form>
	{#if editing === g.id}
		<GoalForm goal={g} onsaved={edited} onconflict={editConflict} oncancel={() => (editing = null)} />
	{/if}
	{#if isActive && confidenceFor?.id === g.id}
		<form class="confidence" aria-label="Record confidence" onsubmit={recordConfidence}>
			{#if g.suggested_confidence}
				<p class="muted">Suggested by {g.suggested_by}: {g.suggested_note || confidenceLabel(g.suggested_confidence)}</p>
			{/if}
			<fieldset disabled={busy}>
				<legend class="visually-hidden">Confidence</legend>
				{#each CONFIDENCES as c (c)}
					<label><input type="radio" name="confidence" value={c} bind:group={choice} /> {confidenceLabel(c)}</label>
				{/each}
			</fieldset>
			<label class="field">Note <textarea bind:value={confidenceNote} rows="2" disabled={busy}></textarea></label>
			<div class="row">
				<button type="submit" class="primary" disabled={busy || !choice}>Record</button>
				<button type="button" class="ghost" onclick={() => (confidenceFor = null)}>Cancel</button>
			</div>
		</form>
	{:else}
		<div class="row">
			{#if isActive}<button type="button" disabled={busy} onclick={() => openConfidence(g)}>Record confidence</button>{/if}
			{#if editing !== g.id}<button type="button" class="ghost" disabled={busy} onclick={() => (editing = g.id)}>Edit</button>{/if}
			{#if !isActive}<button type="button" class="ghost" disabled={busy} onclick={() => remove(g)}>Remove</button>{/if}
		</div>
	{/if}
{/snippet}

<svelte:head><title>Sprint · Reeve</title></svelte:head>

<PageHeader title="Sprint" />

{#if loadError}
	<Notice tone="error" onretry={reload}>Could not load the sprint plan: {loadError}</Notice>
{:else if !sprints}
	<p class="muted">Loading…</p>
{:else}
	{#if staleNotice}
		<Notice tone="error" onretry={reload} ondismiss={() => (staleNotice = null)}>{staleNotice}</Notice>
	{/if}
	{#if notice}
		<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>
	{/if}

	<div class="columns" data-testid="columns">
		<section class="panel" data-testid="active-sprint" aria-labelledby="active-title">
			{#if active}
				{@const progress = new Map(active.counts.goals.map((p) => [p.goal_id, p]))}
				<div class="panel-head">
					<h2 id="active-title">
						This sprint <span class="dates">{active.sprint.starts_on} – {active.sprint.ends_on} · {plural(active.sprint.goals.length, 'goal', 'goals')}</span>
					</h2>
					<button type="button" disabled={busy} onclick={() => (closing = active)}>Close…</button>
				</div>
				<!-- kb/ADR-0014 R2: every view that shows counts states the membership rule. -->
				<p class="rule">{RULE}</p>
				<p class="upkeep muted">{upkeepText(active.counts, active.upkeep_line)}</p>
				{#if active.sprint.warnings.length}
					<p class="warn">Warning: {active.sprint.warnings.map((w) => warningText(w)).join('; ')}</p>
				{/if}
				{#each active.sprint.goals as g (g.id)}
					<GoalRow goal={g} gates={gates.get(g.id)} active progress={progress.get(g.id)} {firstBucket} open={opened.has(g.id)} ontoggle={() => toggle(g.id)}>
						{#snippet controls()}{@render goalControls(g, true)}{/snippet}
					</GoalRow>
				{:else}
					<p class="muted">The active sprint has no goals.</p>
				{/each}
				<div bind:this={activeForm} class="add-slot">
					{#if goalFor === 'active'}
						<GoalForm
							title="New goal for this sprint"
							active
							endsOn={active.sprint.ends_on}
							oncreated={({ goal }) => added(goal.id)}
							oncancel={() => closeGoal('active')}
						/>
					{:else}
						<button type="button" class="ghost add-goal" disabled={busy} onclick={() => openGoal('active')}>+ goal</button>
					{/if}
				</div>
			{:else}
				<h2 id="active-title">This sprint</h2>
				<p class="muted">No active sprint.</p>
			{/if}
		</section>

		<section class="panel" data-testid="next-sprint" aria-labelledby="next-title">
			<div class="panel-head">
				<h2 id="next-title">
					Next sprint {#if draft}<span class="dates">{draft.starts_on} – {draft.ends_on} · {plural(draft.goals.length, 'goal', 'goals')}</span>{/if}
				</h2>
				{#if draft && !active}
					<button type="button" disabled={busy} onclick={openDraft}>Open sprint with {plural(goingIn, 'goal', 'goals')}</button>
				{/if}
			</div>
			{#if draft?.goals.length}<p class="muted">Ticked goals go in when the sprint opens.</p>{/if}
			{#if leaveOut.length}<p class="muted">{plural(leaveOut.length, 'goal stays', 'goals stay')} in Next sprint when it opens.</p>{/if}
			{#if openWarnings.length && active}
				<p class="warn">Opened. Warning: {openWarnings.map((w) => warningText(w)).join('; ')}</p>
			{/if}
			{#if draftWarning.length}
				<p class="warn">Warning: {draftWarning.map((w) => warningText(w)).join('; ')}</p>
			{/if}
			{#each draft?.goals ?? [] as g (g.id)}
				<GoalRow goal={g} gates={gates.get(g.id)} active={false} {firstBucket} open={opened.has(g.id)} ontoggle={() => toggle(g.id)}>
					{#snippet lead()}
						<input
							type="checkbox"
							aria-label="Goes in at open: {g.outcome}"
							title="Goes in at open"
							checked={!leftOut.has(g.id)}
							disabled={busy}
							onchange={(e) => (e.currentTarget.checked ? leftOut.delete(g.id) : leftOut.add(g.id))}
						/>
					{/snippet}
					{#snippet controls()}{@render goalControls(g, false)}{/snippet}
				</GoalRow>
			{/each}
			{#key draft?.id}
				<GoalForm endsOn={draft?.ends_on ?? null} oncreated={() => reload()} />
			{/key}
		</section>
	</div>

	{#if firstBucket !== null}
		<section class="panel" data-testid="bucket-initiatives" aria-labelledby="bucket-title">
			<h2 id="bucket-title">{firstBucket} initiatives</h2>
			{#each plan?.first_bucket_initiatives ?? [] as i (i.key)}
				<div class="bucket-row" data-testid="bucket-{i.key}" bind:this={rows[i.key]}>
					<div class="row">
						<a href={itemRoute({ key: i.key, item: 'initiative' }, page.url)}><span class="key">{i.key}</span> {i.title}</a>
						{#if i.target_month}<span class="muted">{month(i.target_month)}</span>{/if}
						{#if !i.discovery_complete}<span class="muted">in discovery</span>{/if}
						{#each i.goals as id (id)}
							{@const x = everyGoal.get(id)}
							{#if x}<a class="goal-chip" href="#goal-{id}">{x.goal.outcome} · {x.where}</a>{/if}
						{:else}
							<span class="muted">no goal</span>
							{#if goalFor !== i.key}<button type="button" class="ghost add-goal" disabled={busy} onclick={() => openGoal(i.key)}>+ goal</button>{/if}
						{/each}
						{#if i.goals.some((id) => !everyGoal.has(id))}
							<span class="muted more">{plural(i.goals.filter((id) => !everyGoal.has(id)).length, 'goal', 'goals')} not shown</span>
						{/if}
					</div>
					{#if goalFor === i.key}
						<GoalForm
							title="New goal for next sprint · linked to {i.key}"
							links={[i.key]}
							endsOn={draft?.ends_on ?? null}
							oncreated={({ goal }) => added(goal.id)}
							oncancel={() => closeGoal(i.key)}
						/>
					{/if}
				</div>
			{:else}
				<p class="muted">No initiative is in {firstBucket}.</p>
			{/each}
		</section>
	{/if}

	{#if active}
		<details class="panel" data-testid="demo">
			<summary><h2>Done in the last 7 days ({active.demo.length})</h2></summary>
			{#if active.demo.length}
				<ul>
					{#each active.demo as d (d.key)}
						<li><a href="?story={d.key}"><span class="key">{d.key}</span> {d.title}</a></li>
					{/each}
				</ul>
			{:else}
				<p class="muted">Nothing completed in the last 7 days.</p>
			{/if}
		</details>
	{/if}

	<details class="panel" data-testid="history">
		<summary><h2>History ({sprints.closed.length})</h2></summary>
		{#each sprints.closed as s (s.id)}
			<SprintHistoryEntry sprint={s} {refresh} />
		{:else}
			<p class="muted">No closed sprints yet.</p>
		{/each}
	</details>
{/if}

{#if closing}
	<Dialog title="Close the sprint" onclose={() => (closing = null)}>
		<CloseSprint view={closing} {draft} {leaveOut} onchanged={reload} ondone={() => (closing = null)} />
	</Dialog>
{/if}

<style>
	.panel {
		display: grid;
		gap: 6px;
		margin: 0 0 28px;
	}
	.panel-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
	}
	h2,
	p {
		margin: 0;
	}
	.dates {
		color: var(--muted);
		font-size: 14px;
		font-weight: 400;
	}
	.rule {
		color: var(--muted);
		font-size: 13px;
	}
	.confidence {
		display: grid;
		gap: 8px;
		margin-top: 4px;
	}
	fieldset {
		display: flex;
		flex-wrap: wrap;
		gap: 12px;
		margin: 0;
		padding: 0;
		border: 0;
	}
	textarea {
		font: inherit;
	}
	.row,
	.link {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
	}
	.links {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.links li {
		display: inline-flex;
		align-items: center;
		gap: 2px;
	}
	.key {
		font-family: var(--mono);
		font-size: 13px;
	}
	ul {
		margin: 0;
		padding-left: 18px;
	}
	.warn {
		color: var(--warn);
	}
	.ghost.icon {
		height: auto;
		padding: 0 4px;
	}
	.columns {
		display: grid;
		grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
		gap: 16px;
		align-items: start;
	}
	@media (max-width: 900px) {
		.columns {
			grid-template-columns: minmax(0, 1fr);
		}
	}
	.bucket-row {
		display: grid;
		gap: 8px;
	}
	.goal-chip {
		padding: 1px 8px;
		border: 1px solid var(--line);
		border-radius: 999px;
		font-size: 13px;
	}
	summary {
		cursor: pointer;
	}
	/* The folded sections' headings sit inside the summary, so the disclosure marker stays on the same line. */
	summary h2 {
		display: inline;
	}
	.add-goal {
		justify-self: start;
	}
</style>
