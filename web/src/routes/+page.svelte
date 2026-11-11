<!-- web/src/routes/+page.svelte: Home, the landing page. What waits on me (the decisions I decide), what I have started,
     what to pick next (ready work nobody owns, with Start) and the active sprint. Everything comes from four reads the
     server already answers; Home only partitions them (#lib/home.ts) and stores nothing. -->
<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, Conflict, unwrap } from '#lib/api/client.ts';
	import type { AffectedStory, DecisionQueue, DecisionQueueEntry, Reason, Sprints, StoryWithReadiness } from '#lib/api/types.ts';
	import { COLUMN_LABEL } from '#lib/board.ts';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import GoalsStrip from '#lib/components/GoalsStrip.svelte';
	import OverridePrompt from '#lib/components/OverridePrompt.svelte';
	import StoryRow from '#lib/components/StoryRow.svelte';
	import { today } from '#lib/decisions.ts';
	import { decisionHref } from '#lib/drawer.ts';
	import { minuteClock, onLive } from '#lib/events.svelte.ts';
	import { errorMessage } from '#lib/format.ts';
	import { myDecisions, pickNext, sprintLine, underWay } from '#lib/home.ts';
	import { session } from '#lib/me.svelte.ts';
	import { postMove } from '#lib/moves.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';

	let sprints = $state<Sprints | null>(null);
	let queue = $state<DecisionQueue | null>(null);
	let mine = $state<StoryWithReadiness[] | null>(null);
	let ready = $state<StoryWithReadiness[] | null>(null);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);
	/** A Start refused for readiness reasons, waiting for the person's written reason (kb/ADR-0018 R2). */
	let override = $state<{ row: StoryWithReadiness; reasons: Reason[] } | null>(null);
	let showMore = $state(false);

	const loaded = $derived(queue !== null && ready !== null);
	const me = $derived(session.me?.email ?? '');
	/** The viewer's date, re-read every minute, so the sprint line rolls over at midnight without a refetch. */
	const on = $derived(today(new Date(minuteClock.now)));
	const active = $derived(sprints?.active ?? null);

	const decided = $derived(myDecisions(queue?.entries ?? [], me));
	const listed = $derived(decided.listed.map((e) => e.decision));
	const entries = $derived(new Map<number, DecisionQueueEntry>(decided.listed.map((e) => [e.decision.id, e])));
	const blocks = $derived(new Map<number, AffectedStory[]>(decided.listed.map((e) => [e.decision.id, e.blocks])));
	/** Every listed decision after the first three folds behind "n more". */
	const more = $derived(new Set(listed.slice(3).map((d) => d.id)));
	const anyDecision = $derived(decided.listed.length + decided.review + decided.later > 0);

	const work = $derived(underWay(mine ?? []));
	const pick = $derived(pickNext(ready ?? []));
	/** Stories a decision keeps out of active work, each counted once (the queue's own count). */
	const blocked = $derived(queue?.summary.blocked_stories ?? 0);

	const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

	// A write's own reload and a live refetch can overlap; only the latest request's answer (or failure) lands, and a
	// failed reload keeps the last good page.
	let latest = 0;
	async function reload() {
		const seq = ++latest;
		const owner = session.me?.email;
		try {
			const [s, q, m, r] = await Promise.all([
				unwrap(api.GET('/api/v1/sprints')),
				unwrap(api.GET('/api/v1/decisions/queue')),
				owner
					? unwrap(api.GET('/api/v1/stories', { params: { query: { owner, done: 'none' } } }))
					: Promise.resolve([] as StoryWithReadiness[]),
				// Ready already implies todo; `delivery=todo` spares the server every done story.
				unwrap(api.GET('/api/v1/stories', { params: { query: { ready: true, delivery: 'todo' } } }))
			]);
			if (seq !== latest) return;
			sprints = s;
			queue = q;
			mine = m;
			ready = r;
			loadError = null;
		} catch (e) {
			if (seq !== latest) return;
			loadError = errorMessage(e);
		}
	}

	onMount(reload);
	// Any activity entry, and every (re)connect, refetches (kb/ADR-0011 R10, the web's refresh policy).
	onLive(reload);

	/** Start: the board's move to in progress. Starting a story nobody owns makes the mover its owner. */
	async function start(row: StoryWithReadiness, overrideReason?: string) {
		const key = row.story.key;
		const wasFree = row.story.owner == null;
		busy = true;
		notice = null;
		try {
			const out = await postMove(row.story, 'in_progress', overrideReason ? { overrideReason } : {});
			if (out.kind === 'moved') {
				override = null;
				await reload();
				if (wasFree) notice = `${key} is yours now.`;
			} else if (out.kind === 'override') {
				override = { row, reasons: out.reasons };
			} else {
				override = null;
				notice = out.notice;
			}
		} catch (e) {
			override = null;
			if (e instanceof Conflict) {
				notice = `Someone else changed ${key}; Home was reloaded.`;
				await reload();
			} else {
				notice = errorMessage(e);
			}
		} finally {
			busy = false;
		}
	}
</script>

<div class="home">
	<PageHeader title="Home" />
	{#if sprints}
		<p class="sprint-line">
			<span>{active ? sprintLine(active.sprint, on) : 'No sprint is running'}</span>
			<a href="/sprint">Sprint →</a>
		</p>
	{/if}

	{#if loadError}
		<Notice tone="error" onretry={reload}>Could not load Home: {loadError}</Notice>
	{/if}
	{#if notice}
		<Notice ondismiss={() => (notice = null)}>{notice}</Notice>
	{/if}

	{#if loaded}
		{#if anyDecision}
			<section aria-labelledby="home-decisions">
				<div class="head">
					<h2 id="home-decisions">Decisions you decide</h2>
					<a href="/decisions?mine=1">Decisions →</a>
				</div>
				{#if listed.length > 0}
					<DecisionList
						decisions={listed}
						{entries}
						{blocks}
						decisionHref={(id) => decisionHref(page.url, id)}
						onchanged={reload}
						poll={false}
						rest={(d) => more.has(d.id)}
						showRest={showMore}
						restId="home-more-decisions"
						between={more.size > 0 ? fold : undefined}
					/>
				{/if}
				{#if decided.review > 0}
					<p class="line"><a href="/decisions?mine=1#review">{plural(decided.review, 'record', 'records')} to review →</a></p>
				{/if}
				{#if decided.later > 0}
					<p class="line"><a href="/decisions?mine=1">{decided.later} more needed later →</a></p>
				{/if}
			</section>
		{/if}

		<section aria-labelledby="home-under-way">
			<div class="head"><h2 id="home-under-way">Under way</h2></div>
			{#if work.started.length > 0}
				<ul class="rows">
					{#each work.started as row (row.story.key)}<StoryRow {row} />{/each}
				</ul>
			{:else}
				<p class="muted">Nothing under way. Pick something below.</p>
			{/if}
			{#if work.todo > 0}
				<p class="line"><a href="/backlog?mine=1">{work.todo} of yours in todo, {work.ready} ready →</a></p>
			{/if}
		</section>

		<section aria-labelledby="home-pick-next">
			<div class="head"><h2 id="home-pick-next">Pick next</h2></div>
			{#if pick.shown.length > 0}
				<ul class="rows">
					{#each pick.shown as row (row.story.key)}<StoryRow {row} {busy} onstart={(r) => start(r)} />{/each}
				</ul>
			{:else}
				<p class="muted">Nothing ready and unowned.</p>
			{/if}
			<p class="line"><a href="/backlog?ready=1">All ready work →</a></p>
			{#if blocked > 0}
				<p class="line flag">
					<span aria-hidden="true">⚑</span>
					{plural(blocked, 'story waits', 'stories wait')} on a decision. <a href="/board?notready=1">See what is not ready →</a>
				</p>
			{/if}
		</section>

		{#if active}
			<section aria-labelledby="home-sprint">
				<div class="head">
					<h2 id="home-sprint">The sprint</h2>
					<a href="/sprint">Sprint →</a>
				</div>
				<!-- The strip's chips are its filter toggles; on Home choosing one opens the board filtered to it. -->
				<GoalsStrip view={active} selected={null} onselect={(g) => goto(g === null ? '/board' : `/board?goal=${g}`)} />
			</section>
		{/if}
	{:else if !loadError}
		<p class="muted">Loading…</p>
	{/if}
</div>

{#snippet fold()}
	<button type="button" class="ghost" aria-expanded={showMore} aria-controls="home-more-decisions" onclick={() => (showMore = !showMore)}
		>{showMore ? 'Show fewer' : `${more.size} more`}</button
	>
{/snippet}

{#if override}
	{@const o = override}
	<OverridePrompt
		storyKey={o.row.story.key}
		target={COLUMN_LABEL.in_progress}
		reasons={o.reasons}
		{busy}
		onconfirm={(why) => start(o.row, why)}
		oncancel={() => (override = null)}
	/>
{/if}

<style>
	.home {
		display: grid;
		gap: 20px;
		max-width: 55rem;
		margin: 0 auto;
	}
	.sprint-line {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 12px;
		margin: -14px 0 0;
		color: var(--muted);
	}
	section {
		display: grid;
		gap: 8px;
	}
	.head {
		display: flex;
		align-items: baseline;
		gap: 12px;
		border-bottom: 1px solid var(--line);
		padding-bottom: 4px;
	}
	.head h2 {
		font-size: 15px;
		margin: 0;
	}
	.head a {
		margin-left: auto;
		font-size: 13px;
	}
	.rows {
		display: grid;
		gap: 6px;
		margin: 0;
		padding: 0;
	}
	.line {
		margin: 0;
		font-size: 13px;
	}
	.flag {
		color: var(--warn);
	}
	.muted {
		margin: 0;
		color: var(--muted);
	}
</style>
