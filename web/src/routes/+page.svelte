<!-- web/src/routes/+page.svelte: Home, the landing page. What waits on me (the decisions I decide), what I have started,
     what to pick next (ready work nobody owns, with Start) and the active sprint. Everything comes from four reads the
     server already answers; Home only partitions them (#lib/home.ts) and stores nothing. -->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, Conflict, unwrap } from '#lib/api/client.ts';
	import type { DecisionQueue, DecisionQueueEntry, Reason, Sprints, StoryWithReadiness } from '#lib/api/types.ts';
	import { COLUMN_LABEL } from '#lib/board.ts';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import GoalsStrip from '#lib/components/GoalsStrip.svelte';
	import OverridePrompt from '#lib/components/OverridePrompt.svelte';
	import StoryRow from '#lib/components/StoryRow.svelte';
	import { today } from '#lib/decisions.ts';
	import { decisionHref } from '#lib/drawer.ts';
	import { minuteClock, onLive } from '#lib/events.svelte.ts';
	import { errorMessage, plural } from '#lib/format.ts';
	import { myDecisions, pickNext, sprintLine, underWay } from '#lib/home.ts';
	import { session } from '#lib/me.svelte.ts';
	import { postMove } from '#lib/moves.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';

	/** The four reads as the server last answered them, set together; null until the first answer. */
	let data = $state<{ sprints: Sprints; queue: DecisionQueue; mine: StoryWithReadiness[]; ready: StoryWithReadiness[] } | null>(null);
	let loadError = $state<string | null>(null);
	let notice = $state<{ text: string; tone: 'info' | 'error' } | null>(null);
	let busy = $state(false);
	/** A Start refused for readiness reasons, waiting for the person's written reason (kb/ADR-0018 R2). */
	let override = $state<{ row: StoryWithReadiness; reasons: Reason[] } | null>(null);
	let showMore = $state(false);

	/** The viewer's date, re-read every minute, so the sprint line rolls over at midnight without a refetch. */
	const on = $derived(today(new Date(minuteClock.now)));
	const active = $derived(data?.sprints.active ?? null);
	// The layout renders a page only after `session.me` has loaded.
	const decided = $derived(myDecisions(data?.queue.entries ?? [], session.me!.email));
	const listed = $derived(decided.listed.map((e) => e.decision));
	// DecisionList reads each row's held stories from its entry.
	const entries = $derived(new Map<number, DecisionQueueEntry>(decided.listed.map((e) => [e.decision.id, e])));
	/** Every listed decision after the first three folds behind "n more". */
	const more = $derived(new Set(listed.slice(3).map((d) => d.id)));
	const work = $derived(underWay(data?.mine ?? []));
	const pick = $derived(pickNext(data?.ready ?? []));
	/** Todo stories a decision holds, each counted once (the queue's own count). */
	const held = $derived(data?.queue.summary.blocked_stories ?? 0);

	// A write's own reload and a live refetch can overlap; only the latest request's answer (or failure) lands, and a
	// failed reload keeps the last good page.
	let latest = 0;
	async function reload() {
		const seq = ++latest;
		try {
			const [sprints, queue, mine, ready] = await Promise.all([
				unwrap(api.GET('/api/v1/sprints')),
				unwrap(api.GET('/api/v1/decisions/queue')),
				unwrap(api.GET('/api/v1/stories', { params: { query: { owner: session.me!.email, done: 'none' } } })),
				// Ready already implies todo; `delivery=todo` spares the server every done story.
				unwrap(api.GET('/api/v1/stories', { params: { query: { ready: true, delivery: 'todo' } } }))
			]);
			if (seq !== latest) return;
			data = { sprints, queue, mine, ready };
			loadError = null;
		} catch (e) {
			if (seq !== latest) return;
			loadError = errorMessage(e);
		}
	}

	onMount(reload);
	// Any activity entry, and every (re)connect, refetches (kb/ADR-0011 R10, the web's refresh policy).
	onLive(reload);

	/** Start: the board's move to in progress. Pick next lists only stories nobody owns, and starting one makes the mover
	 *  its owner. After the move, focus goes to the story's row under Under way, as the board refocuses a moved card. */
	async function start(row: StoryWithReadiness, overrideReason?: string) {
		const key = row.story.key;
		busy = true;
		notice = null;
		try {
			const out = await postMove(row.story, 'in_progress', { overrideReason });
			if (out.kind === 'override') {
				override = { row, reasons: out.reasons };
				return;
			}
			override = null;
			if (out.kind === 'notice') {
				notice = { text: out.notice, tone: 'error' };
				return;
			}
			await reload();
			notice = { text: `${key} is yours now.`, tone: 'info' };
			await tick();
			document.querySelector<HTMLElement>(`section[aria-labelledby="home-under-way"] [data-key="${key}"] a`)?.focus();
		} catch (e) {
			override = null;
			if (e instanceof Conflict) {
				notice = { text: `Someone else changed ${key}; Home was reloaded.`, tone: 'error' };
				await reload();
			} else {
				notice = { text: errorMessage(e), tone: 'error' };
			}
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head><title>Home · Reeve</title></svelte:head>

<div class="home">
	<PageHeader title="Home">
		{#if data}
			<span class="sprint-line">{active ? sprintLine(active.sprint, on) : 'No sprint is running'}</span>
			<a href="/sprint">Sprint →</a>
		{/if}
	</PageHeader>

	{#if loadError}
		<Notice tone="error" onretry={reload}>Could not load Home: {loadError}</Notice>
	{/if}
	{#if notice}
		<Notice tone={notice.tone} ondismiss={() => (notice = null)}>{notice.text}</Notice>
	{/if}

	{#if data}
		{#if decided.listed.length + decided.review + decided.later > 0}
			<section aria-labelledby="home-decisions">
				<div class="head">
					<h2 id="home-decisions">Decisions you decide</h2>
					<a href="/decisions?mine=1">Readiness →</a>
				</div>
				{#if listed.length > 0}
					<DecisionList
						decisions={listed}
						{entries}
						decisionHref={(id) => decisionHref(page.url, id)}
						onchanged={reload}
						poll={false}
						rest={(d) => more.has(d.id)}
						showRest={showMore}
						restId="home-more-decisions"
					>
						{#snippet between()}
							{#if more.size > 0}
								<button
									type="button"
									class="ghost"
									aria-expanded={showMore}
									aria-controls={showMore ? 'home-more-decisions' : undefined}
									onclick={() => (showMore = !showMore)}>{showMore ? 'Show fewer' : `${more.size} more`}</button
								>
							{/if}
						{/snippet}
					</DecisionList>
				{/if}
				{#if decided.review > 0}
					<p class="line"><a href="/decisions?mine=1">{plural(decided.review, 'record', 'records')} to review →</a></p>
				{/if}
				{#if decided.later > 0}
					<p class="line muted">{decided.later} more not needed in a sprint yet.</p>
				{/if}
			</section>
		{/if}

		<section aria-labelledby="home-under-way">
			<div class="head"><h2 id="home-under-way">Under way</h2></div>
			{#if work.started.length > 0}
				<ul class="stories">
					{#each work.started as row (row.story.key)}<StoryRow {row} />{/each}
				</ul>
			{:else}
				<p class="muted">Nothing under way.</p>
			{/if}
			{#if work.todo > 0}
				<p class="line"><a href="/backlog?mine=1">{work.todo} of yours in todo, {work.ready} ready →</a></p>
			{/if}
		</section>

		<section aria-labelledby="home-pick-next">
			<div class="head"><h2 id="home-pick-next">Pick next</h2></div>
			{#if pick.length > 0}
				<ul class="stories">
					{#each pick as row (row.story.key)}<StoryRow {row} {busy} onstart={start} />{/each}
				</ul>
			{:else}
				<p class="muted">Nothing ready and unowned.</p>
			{/if}
			<p class="line"><a href="/backlog?ready=1">All ready work →</a></p>
			{#if held > 0}
				<p class="line flag">
					<span class="glyph" aria-hidden="true">⚑</span>
					{plural(held, 'story', 'stories')} held by a decision. <a href="/board?notready=1">See what is not ready →</a>
				</p>
			{/if}
		</section>

		{#if active}
			<section aria-labelledby="home-sprint">
				<div class="head">
					<h2 id="home-sprint">The sprint</h2>
					<a href="/sprint">Sprint →</a>
				</div>
				<!-- The strip's chips are its filter toggles; with nothing selected a chip passes its goal, and Home opens the
				     board filtered to it. -->
				<GoalsStrip view={active} selected={null} onselect={(g) => goto(`/board?goal=${g}`)} />
			</section>
		{/if}
	{:else if !loadError}
		<p class="muted">Loading…</p>
	{/if}
</div>

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
	.stories {
		display: grid;
		gap: 6px;
		margin: 0;
		padding: 0;
	}
	.line {
		margin: 0;
		font-size: 13px;
	}
	/* The flag's text stays in the foreground colour on the soft warning ground (an AA pair); only the glyph is amber. */
	.flag {
		padding: 6px 10px;
		border-radius: 6px;
		background: var(--warn-soft);
	}
	.glyph {
		color: var(--warn);
	}
	.muted {
		margin: 0;
		color: var(--muted);
	}
</style>
