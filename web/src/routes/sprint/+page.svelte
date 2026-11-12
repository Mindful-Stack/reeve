<!-- web/src/routes/sprint/+page.svelte: the sprint page, where the PM plans this sprint and the next by goal. The active
     sprint with its goals, numbers, upkeep line and demo list (Check Tuesday: record confidence; Plan Tuesday: Close…);
     the next sprint's goals and links; each goal grouped under its first initiative with what its work waits on
     (`GET /sprints/planning`); the Now initiatives no goal covers yet, each with "+ goal"; and the history, where a closed
     sprint shows its counts at close beside the derived ones when they differ (kb/ADR-0014 R6). -->
<script lang="ts">
	import { tick } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { api, Refused, unwrap } from '#lib/api/client.ts';
	import type { Confidence, Goal, GoalGates as Gates, Initiative, Sprints, SprintView, SprintWarning } from '#lib/api/types.ts';
	import { fullLabel } from '#lib/board.ts';
	import CloseSprint from '#lib/components/CloseSprint.svelte';
	import GoalForm from '#lib/components/GoalForm.svelte';
	import GoalGates from '#lib/components/GoalGates.svelte';
	import SprintHistoryEntry from '#lib/components/SprintHistoryEntry.svelte';
	import { today } from '#lib/decisions.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { dayLabel, errorMessage, month, plural } from '#lib/format.ts';
	import { session } from '#lib/me.svelte.ts';
	import { groupByInitiative, planningOrder } from '#lib/planning.ts';
	import { confidenceLabel, RULE, upkeepText, warningText } from '#lib/sprint.ts';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import { itemRoute } from '#lib/urgency.ts';

	let sprints = $state<Sprints | null>(null);
	/** Each active and draft goal's gates, by goal id. A goal without an entry (the reads raced an opening) shows no
	 *  gates, groups under "Upkeep / no initiative", and its initiative may show under "no goal yet", until the next
	 *  refetch, which the opening's live event triggers. */
	let gates = $state.raw(new Map<number, Gates>());
	let initiatives = $state.raw<Initiative[]>([]);
	let loadError = $state<string | null>(null);
	/** A refetch failed after the page had loaded: what is shown stays; the banner offers a retry. */
	let staleNotice = $state<string | null>(null);
	/** A write the server refused or failed, in words. */
	let notice = $state<string | null>(null);
	let busy = $state(false);

	const active = $derived(sprints?.active ?? null);
	const draft = $derived(sprints?.draft ?? null);

	let latest = 0;
	/** Bumped on every reload, for the history entries. */
	let refresh = $state(0);
	async function reload() {
		const seq = ++latest;
		try {
			// Three reads together: they land or fail together, so the page never shows a partial state.
			const [s, g, i] = await Promise.all([
				unwrap(api.GET('/api/v1/sprints')),
				unwrap(api.GET('/api/v1/sprints/planning')),
				unwrap(api.GET('/api/v1/initiatives'))
			]);
			if (seq !== latest) return;
			// A new draft (an opening happened): nothing is unticked in it.
			if (s.draft?.id !== sprints?.draft?.id) {
				leftOut.clear();
				// The "+ goal" form is for the draft it was opened on: a new draft never receives its half-typed goal.
				goalFor = null;
			}
			sprints = s;
			gates = new Map(g.map((x) => [x.goal_id, x]));
			initiatives = i;
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

	// --- Planning by goal ------------------------------------------------------------------------------------------
	const buckets = $derived(session.config?.horizon_buckets ?? []);
	/** A sprint's goals under the first of their initiatives, in the planning order (`groupByInitiative`), goals with
	 *  none last; only groups holding a goal. */
	function groups(goals: Goal[]) {
		return groupByInitiative(buckets, initiatives, goals, (g) => gates.get(g.id)?.initiatives[0] ?? null, 'Upkeep / no initiative').filter(
			(x) => x.items.length > 0
		);
	}
	/** The first configured horizon bucket (kb/ADR-0016 R2); without one the "no goal yet" section is absent. */
	const nowBucket = $derived(buckets[0] ?? null);
	/** Its initiatives that no active or draft goal covers, in `/planning`'s order for a bucket (target month, none last,
	 *  then title). Urgency never orders, filters or shows here (kb/ADR-0024 R7). */
	const noGoalYet = $derived.by(() => {
		if (nowBucket === null) return [];
		const covered = new Set([...gates.values()].flatMap((x) => x.initiatives));
		return planningOrder([nowBucket], initiatives.filter((i) => i.horizon === nowBucket && !covered.has(i.key)));
	});
	/** The initiative whose "+ goal" form is open. */
	let goalFor = $state<string | null>(null);
	/** The "no goal yet" rows, by initiative key, to put focus inside them. */
	const rows: Record<string, HTMLElement | undefined> = {};
	/** Opens the form and focuses its outcome input once it is in the DOM. */
	async function openGoal(key: string) {
		goalFor = key;
		await tick();
		rows[key]?.querySelector<HTMLInputElement>('input[name="outcome"]')?.focus();
	}
	/** Cancels the form and returns focus to the row's "+ goal" button. */
	async function closeGoal(key: string) {
		goalFor = null;
		await tick();
		rows[key]?.querySelector<HTMLButtonElement>('button.add-goal')?.focus();
	}
	/** A goal's creation as the browser's local day, in the app's day words ("3 Nov"): `today` gives the local
	 *  `YYYY-MM-DD` of an instant, `dayLabel` words it, so no locale is assumed. */
	const waitingSince = (iso: string) => dayLabel(today(new Date(iso)));
</script>

<!-- A goal's editing controls, the same on the active and the draft panel: its epic and initiative links (stories are
     linked from the drawer), and Edit. -->
{#snippet goalControls(g: Goal)}
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
{/snippet}

<!-- A group header: the initiative's key and title, or the upkeep title. -->
{#snippet groupHead(key: string | null, title: string)}
	<h3 class="group-head">{#if key}<span class="key">{key}</span>{title === key ? '' : ` ${title}`}{:else}{title}{/if}</h3>
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

	{#if active}
		{@const progress = new Map(active.counts.goals.map((p) => [p.goal_id, p]))}
		<section class="panel" data-testid="active-sprint" aria-labelledby="active-title">
			<div class="panel-head">
				<h2 id="active-title">Active sprint <span class="dates">{active.sprint.starts_on} – {active.sprint.ends_on}</span></h2>
				<button type="button" disabled={busy} onclick={() => (closing = active)}>Close…</button>
			</div>
			<!-- kb/ADR-0014 R2: every view that shows counts states the membership rule. -->
			<p class="rule">{RULE}</p>
			{#if active.sprint.warnings.length}
				<p class="warn">Warning: {active.sprint.warnings.map((w) => warningText(w)).join('; ')}</p>
			{/if}
			{#each groups(active.sprint.goals) as grp (grp.key)}
				<div class="group" data-testid="active-group-{grp.key ?? 'none'}">
					{@render groupHead(grp.key, grp.title)}
					{#each grp.items as g (g.id)}
						{@const p = progress.get(g.id)}
						{@const gg = gates.get(g.id)}
						<article class="goal" data-testid="goal-{g.id}">
							<h4>{g.outcome}</h4>
							<p>We'll know when: {g.check}</p>
							<p class="muted">Owner: {g.owner}</p>
							{#if g.notes}<p class="notes">{g.notes}</p>{/if}
							<p>
								Confidence: <span data-testid="confidence">{g.confidence ? confidenceLabel(g.confidence) : 'not recorded'}</span>
								{#if g.confidence_note}<span class="muted">({g.confidence_note})</span>{/if}
							</p>
							{#if g.suggested_confidence}
								<p class="suggestion muted" data-testid="suggestion">
									suggested: {confidenceLabel(g.suggested_confidence)} by {g.suggested_by}{#if g.suggested_note}{' '}({g.suggested_note}){/if}
								</p>
							{/if}
							<p class="counts">
								{p?.completed ?? 0} completed · {p?.in_flight ?? 0} in flight ({p?.in_review ?? 0} in review) · {p?.todo ?? 0} todo
							</p>
							{#if gg}<GoalGates gates={gg} linked={g.links.length > 0} />{/if}
							{@render goalControls(g)}
							{#if confidenceFor?.id === g.id}
								<form class="confidence" aria-label="Record confidence" onsubmit={recordConfidence}>
									{#if g.suggested_confidence}
										<p class="muted">
											Suggested by {g.suggested_by}: {g.suggested_note || confidenceLabel(g.suggested_confidence)}
										</p>
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
									<button type="button" disabled={busy} onclick={() => openConfidence(g)}>Record confidence</button>
									{#if editing !== g.id}<button type="button" class="ghost" disabled={busy} onclick={() => (editing = g.id)}>Edit</button>{/if}
								</div>
							{/if}
						</article>
					{/each}
				</div>
			{:else}
				<p class="muted">The active sprint has no goals.</p>
			{/each}
			<p class="upkeep">{upkeepText(active.counts, active.upkeep_line)}</p>
			<div data-testid="demo">
				<h3>done in the last 7 days</h3>
				{#if active.demo.length}
					<ul>
						{#each active.demo as d (d.key)}
							<li><a href="?story={d.key}"><span class="key">{d.key}</span> {d.title}</a></li>
						{/each}
					</ul>
				{:else}
					<p class="muted">Nothing completed in the last 7 days.</p>
				{/if}
			</div>
		</section>
	{/if}

	<section class="panel" data-testid="next-sprint" aria-labelledby="next-title">
		<div class="panel-head">
			<h2 id="next-title">Next sprint {#if draft}<span class="dates">{draft.starts_on} – {draft.ends_on}</span>{/if}</h2>
			{#if draft && !active}
				<button type="button" disabled={busy} onclick={openDraft}>Open sprint with {plural(goingIn, 'goal', 'goals')}</button>
			{/if}
		</div>
		{#if leaveOut.length}<p class="muted">{plural(leaveOut.length, 'goal stays', 'goals stay')} in Next sprint when it opens.</p>{/if}
		{#if openWarnings.length && active}
			<p class="warn">Opened. Warning: {openWarnings.map((w) => warningText(w)).join('; ')}</p>
		{/if}
		{#if draftWarning.length}
			<p class="warn">Warning: {draftWarning.map((w) => warningText(w)).join('; ')}</p>
		{/if}
		{#each groups(draft?.goals ?? []) as grp (grp.key)}
			<div class="group" data-testid="next-group-{grp.key ?? 'none'}">
				{@render groupHead(grp.key, grp.title)}
				{#each grp.items as g (g.id)}
					{@const gg = gates.get(g.id)}
					<article class="goal draft" data-testid="goal-{g.id}">
						<h4>{g.outcome}</h4>
						<p class="muted">Waiting since {waitingSince(g.created_at)}</p>
						<label class="goes-in"
							><input
								type="checkbox"
								aria-label="Goes in at open: {g.outcome}"
								checked={!leftOut.has(g.id)}
								disabled={busy}
								onchange={(e) => (e.currentTarget.checked ? leftOut.delete(g.id) : leftOut.add(g.id))}
							/> Goes in at open</label
						>
						<p>We'll know when: {g.check}</p>
						<p class="muted">Owner: {g.owner}</p>
						{#if g.notes}<p class="notes">{g.notes}</p>{/if}
						{#if gg}<GoalGates gates={gg} linked={g.links.length > 0} />{/if}
						{@render goalControls(g)}
						<div class="row">
							{#if editing !== g.id}<button type="button" class="ghost" disabled={busy} onclick={() => (editing = g.id)}>Edit</button>{/if}
							<button type="button" class="ghost" disabled={busy} onclick={() => remove(g)}>Remove</button>
						</div>
					</article>
				{/each}
			</div>
		{/each}
		{#key draft?.id}
			<GoalForm endsOn={draft?.ends_on ?? null} oncreated={() => reload()} />
		{/key}
	</section>

	{#if nowBucket !== null}
		<section class="panel" data-testid="no-goal-yet" aria-labelledby="no-goal-title">
			<h2 id="no-goal-title">{nowBucket}, no goal yet</h2>
			{#each noGoalYet as i (i.key)}
				<div class="now-row" data-testid="no-goal-{i.key}" bind:this={rows[i.key]}>
					<div class="row">
						<a href={itemRoute({ key: i.key, item: 'initiative' }, page.url)}><span class="key">{i.key}</span> {i.title}</a>
						{#if i.target_month}<span class="muted">{month(i.target_month)}</span>{/if}
						{#if !i.discovery_complete}<span class="muted">in discovery</span>{/if}
						{#if goalFor !== i.key}<button type="button" class="ghost add-goal" disabled={busy} onclick={() => openGoal(i.key)}>+ goal</button>{/if}
					</div>
					{#if goalFor === i.key}
						<GoalForm
							title="New goal for next sprint · linked to {i.key}"
							links={[i.key]}
							endsOn={draft?.ends_on ?? null}
							oncreated={async () => {
								goalFor = null;
								await reload();
							}}
							oncancel={() => closeGoal(i.key)}
						/>
					{/if}
				</div>
			{:else}
				<p class="muted">Every {nowBucket} initiative has a goal this sprint or next.</p>
			{/each}
		</section>
	{/if}

	<section class="panel" aria-labelledby="history-title">
		<h2 id="history-title">History</h2>
		{#each sprints.closed as s (s.id)}
			<SprintHistoryEntry sprint={s} {refresh} />
		{:else}
			<p class="muted">No closed sprints yet.</p>
		{/each}
	</section>
{/if}

{#if closing}
	<Dialog title="Close the sprint" onclose={() => (closing = null)}>
		<CloseSprint view={closing} {draft} {leaveOut} onchanged={reload} ondone={() => (closing = null)} />
	</Dialog>
{/if}

<style>
	.goes-in {
		display: flex;
		gap: 6px;
		align-items: center;
		font-size: 13px;
	}
	.panel {
		display: grid;
		gap: 10px;
		margin: 0 0 28px;
	}
	.panel-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
	}
	h2,
	h3,
	h4,
	p {
		margin: 0;
	}
	.dates {
		color: var(--muted);
		font-size: 14px;
		font-weight: 400;
	}
	.group {
		display: grid;
		gap: 8px;
	}
	/* app.css sizes only h1 and h2: the page's h3 (group headers, the demo list's heading) sit above the goals' h4. */
	h3 {
		font-size: 16px;
	}
	h4 {
		font-size: 15px;
	}
	.goal.draft {
		border-style: dashed;
	}
	.now-row {
		display: grid;
		gap: 8px;
	}
	.rule {
		color: var(--muted);
		font-size: 13px;
	}
	.goal {
		display: grid;
		gap: 4px;
		padding: 10px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.goal button,
	.goal form {
		justify-self: start;
	}
	.notes {
		white-space: pre-wrap;
	}
	.counts {
		font-variant-numeric: tabular-nums;
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
</style>
