<!-- web/src/lib/components/SprintHistoryEntry.svelte: one closed sprint in the sprint page's history: its goals,
     confidence and verdicts, and on demand its numbers, derived on read (`GET /sprints/{id}`) beside its counts at close
     (the `closed` activity entry) wherever a number differs (kb/ADR-0014 R6). -->
<script lang="ts">
	import { api, unwrap } from '#lib/api/client.ts';
	import type { ActivityEntry, GoalProgress, Sprint, SprintCounts, SprintView } from '#lib/api/types.ts';
	import { errorMessage } from '#lib/format.ts';
	import { blockedText, confidenceLabel, countsDiffer, nowThen, RULE, upkeepTextAt, verdictLabel } from '#lib/sprint.ts';
	import Notice from '#lib/ui/Notice.svelte';

	let {
		sprint,
		refresh = 0
	}: {
		/** The closed sprint, as `GET /sprints` lists it. */
		sprint: Sprint;
		/** Bumped by the page on every reload: open numbers refetch, since they are derived on read too. */
		refresh?: number;
	} = $props();

	type Detail = { view: SprintView; atClose: SprintCounts | null } | { error: string } | 'loading';
	/** Null while the numbers are folded. */
	let detail = $state<Detail | null>(null);

	/** The counts at close from a `closed` entry's payload, or null when there is none to read. */
	function countsOf(entries: ActivityEntry[]): SprintCounts | null {
		const c = (entries[0]?.payload as { counts?: SprintCounts } | null)?.counts;
		return c && Array.isArray(c.goals) ? c : null;
	}

	/** Only the newest load may write: an older one that answers later (a fold and unfold, a reload) is dropped. */
	let latest = 0;
	async function load() {
		const seq = ++latest;
		const id = sprint.id;
		try {
			const [view, entries] = await Promise.all([
				unwrap(api.GET('/api/v1/sprints/{id}', { params: { path: { id } } })),
				unwrap(api.GET('/api/v1/activity', { params: { query: { entity_type: 'sprint', entity_key: String(id), action: 'closed' } } }))
			]);
			if (seq === latest && detail !== null) detail = { view, atClose: countsOf(entries) };
		} catch (e) {
			if (seq === latest && detail !== null) detail = { error: errorMessage(e) };
		}
	}

	function toggle() {
		if (detail !== null) {
			detail = null;
			latest++; // a load in flight is now stale
		} else {
			detail = 'loading';
			void load();
		}
	}

	// A page reload refetches the numbers while they are open (not on mount: they start folded).
	let seen = 0;
	$effect(() => {
		const r = refresh;
		if (r === seen) return;
		seen = r;
		if (detail !== null) void load();
	});

	const numbers = (p: Pick<GoalProgress, 'completed' | 'in_flight' | 'in_review' | 'todo'>, t?: GoalProgress) =>
		`${nowThen(p.completed, t?.completed)} completed · ${nowThen(p.in_flight, t?.in_flight)} in flight (${nowThen(p.in_review, t?.in_review)} in review) · ${nowThen(p.todo, t?.todo)} todo`;
</script>

<article class="closed" data-testid="history-{sprint.id}">
	<div class="head">
		<h3>Sprint {sprint.id} <span class="dates">{sprint.starts_on} – {sprint.ends_on}</span></h3>
		<button type="button" class="ghost" aria-expanded={detail !== null} onclick={toggle}
			>{detail === null ? 'Show numbers' : 'Hide numbers'}</button
		>
	</div>
	<ul>
		{#each sprint.goals as g (g.id)}
			<li>
				<span class="outcome">{g.outcome}</span> · confidence {g.confidence ? confidenceLabel(g.confidence) : 'not recorded'} ·
				verdict {g.verdict ? verdictLabel(g.verdict) : 'none'}{#if g.verdict_note}{' '}({g.verdict_note}){/if}
			</li>
		{/each}
	</ul>
	{#if detail === 'loading'}
		<p class="muted">Loading…</p>
	{:else if detail && 'error' in detail}
		<Notice tone="error" onretry={load}>Could not load the numbers: {detail.error}</Notice>
	{:else if detail}
		{@const d = detail}
		<!-- kb/ADR-0014 R6: the entry's counts show beside the derived ones only where a number differs; blocked-by-
		     decisions is the value at close alone, never recomputed, and never part of that comparison. -->
		{@const atClose = countsDiffer(d.view.counts, d.atClose) ? d.atClose : null}
		{@const now = new Map(d.view.counts.goals.map((p) => [p.goal_id, p]))}
		{@const entry = new Map((d.atClose?.goals ?? []).map((p) => [p.goal_id, p]))}
		<div class="numbers" data-testid="numbers">
			<p class="rule">{RULE}</p>
			<!-- A closed sprint always has its entry (the CLI calls a missing one an error); without it, say so, and show
			     no blocked-by value, since the live one is today's, not the sprint's. -->
			{#if d.atClose === null}<p class="warn" data-testid="no-close-record">no record of the counts at close</p>{/if}
			{#if atClose}<p class="warn">changed since close: items moved between parents or reopened</p>{/if}
			{#each d.view.sprint.goals as g (g.id)}
				{@const p = now.get(g.id)}
				{@const t = atClose ? entry.get(g.id) : undefined}
				{@const e = entry.get(g.id)}
				<p data-testid="numbers-{g.id}">
					<span class="outcome">{g.outcome}</span>:
					{#if p}
						{numbers(p, t)}{#if e}{' '}· {blockedText(e.blocked_decisions)} at close{/if}{#if atClose && !t}{' '}(not at close){/if}
					{:else if t}
						(removed since close) at close: {numbers(t)}
					{/if}
				</p>
			{/each}
			{#each (atClose?.goals ?? []).filter((x) => !d.view.sprint.goals.some((g) => g.id === x.goal_id)) as x (x.goal_id)}
				<p>Goal {x.goal_id} (removed since close) at close: {numbers(x)}</p>
			{/each}
			<p>{upkeepTextAt(d.view.counts, atClose, d.view.upkeep_line)}</p>
			{#if d.view.demo.length}
				<h4>done in the last 7 days</h4>
				<ul>
					{#each d.view.demo as x (x.key)}
						<li><a href="?story={x.key}"><span class="key">{x.key}</span> {x.title}</a></li>
					{/each}
				</ul>
			{/if}
		</div>
	{/if}
</article>

<style>
	.closed {
		display: grid;
		gap: 4px;
		padding: 10px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
	}
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
	ul {
		margin: 0;
		padding-left: 18px;
	}
	.outcome {
		font-weight: 600;
	}
	.numbers {
		display: grid;
		gap: 4px;
		margin-top: 6px;
		font-variant-numeric: tabular-nums;
	}
	.rule {
		color: var(--muted);
		font-size: 13px;
	}
	.warn {
		color: var(--warn);
	}
	.key {
		font-family: var(--mono);
		font-size: 13px;
	}
</style>
