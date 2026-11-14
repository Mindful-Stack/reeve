<script lang="ts">
	import Glyph from '#lib/components/Glyph.svelte';
	import { levelOfItem, levelOfKind } from '#lib/glyphs.ts';
	import { onMount, tick } from 'svelte';
	import { page } from '$app/state';
	import { api, ApiError, unwrap } from '#lib/api/client.ts';
	import type { AffectedStory, DecisionQueueEntry, NeededIn } from '#lib/api/types.ts';
	import { COLUMN_LABEL } from '#lib/board.ts';
	import { dateLine, isNowReason, neededLabel, nextStepText, REVERSIBILITY_TEXT, shortSha, today } from '#lib/decisions.ts';
	import { storyHref } from '#lib/drawer.ts';
	import { everyVisible, minuteClock, onLive } from '#lib/events.svelte.ts';
	import { errorMessage, short } from '#lib/format.ts';
	import { safePrUrl } from '#lib/prs.ts';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import NeededInReason from '#lib/components/NeededInReason.svelte';
	import UrgentChip from '#lib/components/UrgentChip.svelte';
	import Notice from '#lib/ui/Notice.svelte';

	// One decision, over whatever view is open: the layout mounts this while `?decision=ID` is in the URL (keyed by ID).
	// It shows when the decision is needed, what it needs next, its record and the work it holds, and renders the decision
	// itself (thread, Edit, Close, the mismatch warning) through `DecisionList` with its discussion open, so there is one
	// implementation of those. Nothing here writes: every change goes through the list and reloads this.
	let { decisionId, onclose }: { decisionId: number; onclose: () => void } = $props();

	let entry = $state<DecisionQueueEntry | null>(null);
	let missing = $state(false);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	/** A reload failed after the decision had loaded: the content stays, the notice offers a retry. */
	let reloadFailed = $state(false);
	let heading = $state<HTMLElement>();
	let latest = 0;

	/** Every change is followed by this; an older answer that arrives after a newer one is dropped. */
	async function reload() {
		const id = decisionId;
		const seq = ++latest;
		try {
			const loaded = await unwrap(api.GET('/api/v1/decisions/{id}/context', { params: { path: { id } } }));
			if (seq !== latest) return;
			entry = loaded;
			missing = false;
			loadError = null;
			notice = null;
			reloadFailed = false;
		} catch (e) {
			if (seq !== latest) return;
			if (e instanceof ApiError && e.status === 404) {
				missing = true;
				entry = null;
			} else if (entry) {
				notice = `Could not reload decision #${id}: ${errorMessage(e)}`;
				reloadFailed = true;
			} else loadError = errorMessage(e);
		}
	}

	// The layout owns the opener: it returns focus there once the drawer closes.
	onMount(() => {
		void reload().then(async () => {
			await tick();
			heading?.focus();
		});
	});

	// Any activity entry, and every (re)connect, refetches (#lib/events.svelte.ts). A scheduled check that finds what the last
	// one found logs nothing, so while a record is linked and the decision is open it also refetches every two minutes.
	onLive(reload);
	everyVisible(() => {
		if (entry?.decision.record && !entry.decision.closed) void reload();
	}, 2 * 60_000);

	/** Where the decision's item opens: its page, or the story drawer over this view. */
	function itemLink(e: DecisionQueueEntry): string {
		const d = e.decision;
		const k = encodeURIComponent(d.on);
		if (d.on_type === 'initiative') return `/initiatives/${k}`;
		if (d.on_type === 'epic') return `/epics/${k}`;
		return storyHref(page.url, d.on);
	}

	const GROUPS: { key: Exclude<NeededIn, 'none'>; label: string }[] = [
		{ key: 'this_sprint', label: 'This sprint' },
		{ key: 'next_sprint', label: 'Next sprint' },
		{ key: 'beyond', label: 'Beyond' }
	];
	/** The held stories by the window they are needed in, empty groups left out. The server never sends `none` for a held
	 *  story (it is held by a decision some goal, discovery or promise waits on), so `none` is folded into Beyond rather than given a group. */
	function grouped(stories: AffectedStory[]): { key: NeededIn; label: string; stories: AffectedStory[] }[] {
		const window = (s: AffectedStory) => (s.needed_in === 'none' ? 'beyond' : s.needed_in);
		return GROUPS.map((g) => ({ ...g, stories: stories.filter((s) => window(s) === g.key) })).filter((g) => g.stories.length);
	}

	/** An open, unsatisfied decision no goal, discovery or promise waits on yet: the strip says so instead of staying empty. */
	function notNeeded(e: DecisionQueueEntry): boolean {
		return e.needed_in === 'none' && !e.decision.closed && e.decision.check.kind !== 'accepted';
	}

	const list = $derived(entry ? [entry.decision] : []);

	function keydown(e: KeyboardEvent) {
		// Escape in a form field is the field's own (cancel an edit); it never closes the drawer over an unsaved draft.
		const t = e.target;
		const inField = t instanceof HTMLElement && t.matches('textarea, input, select');
		if (e.key === 'Escape' && !e.defaultPrevented && !inField) {
			e.preventDefault();
			onclose();
		}
	}
</script>

{#snippet storyRow(s: AffectedStory)}
	<li>
		<Glyph level={levelOfKind(s.kind)} /><a class="key" data-sveltekit-reset="false" href={storyHref(page.url, s.key)}>{s.key}</a>
		<span class="story-title">{s.title}</span>
		{#if s.urgent}<UrgentChip line="Urgent" />{/if}
		{#if s.owner}<span class="muted">{short(s.owner)}</span>{/if}
		{#if s.delivery !== 'todo'}<span class="chip">{COLUMN_LABEL[s.delivery]}</span>{/if}
	</li>
{/snippet}

<!-- The explicit role is kept for selectors and tests that look the drawer up by it. -->
<!-- Escape anywhere inside closes it, so the listener sits on the panel. -->
<!-- svelte-ignore a11y_no_redundant_roles, a11y_no_noninteractive_element_interactions -->
<aside class="drawer" role="complementary" aria-label="Decision #{decisionId}" data-testid="decision-drawer" onkeydown={keydown}>
	<header class="top">
		<h2 tabindex="-1" bind:this={heading}>{#if entry}<span class="h-title">{entry.decision.title}</span>{' '}<span class="muted">#{decisionId}</span>{:else}Decision #{decisionId}{/if}</h2>
		{#if entry}
			<span class="item muted"
				>on {entry.decision.on_type}
				<a class="key" data-testid="item-link" href={itemLink(entry)}><Glyph level={levelOfItem(entry.decision.on_type)} />{entry.decision.on}</a></span
			>
		{/if}
		<button type="button" class="close" aria-label="Close decision #{decisionId}" onclick={onclose}>×</button>
	</header>

	{#if missing}
		<p class="muted" data-testid="missing">No decision #{decisionId}</p>
	{:else if loadError}
		<p class="error" role="alert">Could not load decision #{decisionId}: {loadError}</p>
	{:else if !entry}
		<p class="muted">Loading…</p>
	{:else}
		{@const e = entry}
		{@const d = e.decision}
		{@const next = e.next_step}
		{@const pr = d.pr_url ? safePrUrl(d.pr_url) : null}
		{@const stripPr = next?.kind === 'review_record' ? safePrUrl(next.pr_url) : null}
		{@const when = dateLine(d, today(new Date(minuteClock.now)))}

		{#if notice}
			<Notice
				onretry={reloadFailed ? reload : undefined}
				ondismiss={() => {
					notice = null;
					reloadFailed = false;
				}}>{notice}</Notice
			>
		{/if}

		{#if e.needed_in !== 'none' || notNeeded(e) || next}
			<div class="strip" data-testid="strip">
				{#if e.needed_in !== 'none'}
					<p class="needed">
						{#if e.needed_in_reason && isNowReason(e.needed_in_reason)}<NeededInReason reason={e.needed_in_reason} row={{ kind: 'decision', entry: e }} />{:else}<strong>{neededLabel(e.needed_in)}</strong>{#if e.needed_in_reason}{' · '}<NeededInReason reason={e.needed_in_reason} row={{ kind: 'decision', entry: e }} />{/if}{/if}{#if e.under}{' · '}under {e.under}, in discovery{/if}
					</p>
				{:else if notNeeded(e)}
					<p class="needed"><strong>{neededLabel('none')}</strong></p>
				{/if}
				{#if next}
					<p class="next-step">
						{#if stripPr}<a href={stripPr} target="_blank" rel="noopener noreferrer">{nextStepText(next, d.reversibility, minuteClock.now)}</a>{:else}{nextStepText(next, d.reversibility, minuteClock.now)}{/if}
					</p>
				{/if}
				<p class="who muted" data-testid="who">
					{short(d.decider)} decides · {when.text}{#if d.reversibility}{' · '}{REVERSIBILITY_TEXT[d.reversibility]}{/if}
				</p>
			</div>
		{/if}

		{#if d.record || (pr && !stripPr) || d.closed || d.check.kind === 'accepted'}
			<div class="record-block" data-testid="record">
				{#if d.record}
					{@const url = d.record.browse_url ? safePrUrl(d.record.browse_url) : null}
					<p>
						<span class="muted">Record</span>
						{#if url}<a class="mono" href={url} target="_blank" rel="noopener noreferrer">{d.record.repo}/{d.record.path}</a>{:else}<span class="mono">{d.record.repo}/{d.record.path}</span>{/if}
					</p>
				{/if}
				<!-- The strip already links the PR of a record awaiting review. -->
				{#if pr && !stripPr}<p><a href={pr} target="_blank" rel="noopener noreferrer">Linked PR</a></p>{/if}
				{#if d.check.kind === 'accepted'}<p>Satisfied by its record ({shortSha(d.check.commit)})</p>{/if}
				{#if d.closed}<p>Closed{#if d.closed.kind}{' · '}{d.closed.kind}{/if}</p>{/if}
			</div>
		{/if}

		<section class="held" aria-labelledby="held-{decisionId}">
			<h3 id="held-{decisionId}">Held work</h3>
			{#if !e.blocks.length && !e.under_way.length}
				<p class="muted">{d.closed || d.check.kind === 'accepted' ? 'Holds nothing' : 'Holds nothing yet'}</p>
			{/if}
			{#each grouped(e.blocks) as g (g.key)}
				<h4>{g.label}</h4>
				<ul class="stories" data-group={g.key}>
					{#each g.stories as s (s.key)}{@render storyRow(s)}{/each}
				</ul>
			{/each}
			{#if e.under_way.length}
				<h3>Under way without it</h3>
				<ul class="stories" data-group="under-way">
					{#each e.under_way as s (s.key)}{@render storyRow(s)}{/each}
				</ul>
			{/if}
		</section>

		<!-- The row carries the title, decider, date, status, Edit and Close, the mismatch warning and the discussion. It gets
		     no queue entry: the strip above already says the next step, and the held work sits above it. The discussion comes last, so a long
		     thread never pushes the held work off-screen. -->
		<DecisionList decisions={list} expandAll onchanged={reload} poll={false} />
	{/if}
</aside>

<style>
	.drawer {
		position: fixed;
		top: var(--top-bar-height, 0px);
		right: 0;
		bottom: 0;
		z-index: 10;
		display: grid;
		/* One column no wider than the panel: an input's intrinsic width must not widen the track into a horizontal scroll. */
		grid-template-columns: minmax(0, 1fr);
		align-content: start;
		gap: 16px;
		width: min(560px, 100%);
		box-sizing: border-box;
		padding: 16px 20px 28px;
		overflow-x: hidden;
		overflow-y: auto;
		scrollbar-width: thin;
		border-left: 1px solid var(--line);
		background: var(--background);
		box-shadow: var(--shadow);
	}
	.top {
		display: flex;
		gap: 0.5rem;
		justify-content: space-between;
		align-items: center;
		padding: 0;
		border: 0;
		background: none;
	}
	.top .item {
		min-width: 0;
		margin-left: auto;
		margin-right: 8px;
		overflow: hidden;
		font-size: 14px;
		white-space: nowrap;
		text-overflow: ellipsis;
	}
	h2 {
		margin: 0;
		font-size: 18px;
	}
	h3 {
		margin: 0 0 6px;
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--muted);
	}
	h4 {
		margin: 8px 0 4px;
		font-size: 14px;
	}
	.close {
		font-size: 18px;
		line-height: 1;
	}
	.strip,
	.record-block {
		display: grid;
		gap: 4px;
		padding: 10px 12px;
		border-radius: 6px;
		background: var(--lighter-background);
		font-size: 14px;
	}
	.strip p,
	.record-block p {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.mono {
		font-family: var(--mono);
	}
	.stories {
		display: grid;
		gap: 6px;
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: 14px;
	}
	.stories li {
		display: flex;
		flex-wrap: wrap;
		gap: 2px 8px;
		align-items: baseline;
	}
	.story-title {
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.chip {
		padding: 0 0.4rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		font-size: 13px;
		color: var(--muted);
	}
	.muted {
		margin: 0;
		color: var(--muted);
		font-size: 14px;
	}
	.error {
		color: var(--danger);
	}
</style>
