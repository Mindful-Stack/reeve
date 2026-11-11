<script lang="ts">
	import type { StoryWithReadiness } from '#lib/api/types.ts';
	import { COLUMN_LABEL, COLUMNS, fullLabel, storyStatus, type Column } from '#lib/board.ts';
	import { freshNote, live } from '#lib/events.svelte.ts';
	import { session } from '#lib/me.svelte.ts';
	import { prLine, prSummary, safePrUrl } from '#lib/prs.ts';
	import { truncate } from '#lib/sprint.ts';
	import ActionsMenu, { type MenuItem } from '#lib/ui/ActionsMenu.svelte';
	import Avatar from '#lib/ui/Avatar.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';
	import UrgentChip from './UrgentChip.svelte';
	import { urgentLine } from '#lib/urgency.ts';

	let {
		item,
		epicTitle,
		href,
		ondragstart,
		ondragend,
		busy = false,
		onmove,
		onup,
		ondown,
		onresolve
	}: {
		item: StoryWithReadiness;
		/** The epic's title, or undefined for an untriaged story. */
		epicTitle?: string;
		/** Where the title links: the current URL with `?story=KEY` (the drawer). */
		href: string;
		ondragstart?: (e: DragEvent) => void;
		ondragend?: (e: DragEvent) => void;
		/** Disables every menu item that writes while a write is in flight. */
		busy?: boolean;
		/** The "Move to" items: the same path as a drop on another column. */
		onmove?: (to: Column) => void;
		/** Rank items on ready up-next cards: undefined hides the item, null disables it (the card is at that edge). */
		onup?: (() => void) | null;
		ondown?: (() => void) | null;
		/** A done card that can be re-resolved (completed -> dropped): opens the resolution dialog. */
		onresolve?: () => void;
	} = $props();

	const s = $derived(item.story);
	const pr = $derived(prSummary(s.prs));
	// The badge links to the pull request that decided its status.
	const prLink = $derived(s.prs.find((p) => p.status === pr.status));
	const prHref = $derived(prLink ? safePrUrl(prLink.url) : null);
	const prAria = $derived(
		s.prs.length === 1 && prLink
			? `pull request ${prLink.repo}#${prLink.number}, ${pr.status}`
			: `${pr.label}, most advanced is ${pr.status}`
	);

	const items = $derived<MenuItem[]>([
		...(onmove
			? COLUMNS.filter((c) => c !== s.delivery).map((c) => ({
					label: `Move to ${COLUMN_LABEL[c]}`,
					onselect: () => onmove(c),
					disabled: busy
				}))
			: []),
		...(onup !== undefined ? [{ label: 'Move up', onselect: () => onup?.(), disabled: busy || !onup }] : []),
		...(ondown !== undefined ? [{ label: 'Move down', onselect: () => ondown?.(), disabled: busy || !ondown }] : []),
		...(onresolve ? [{ label: 'Change resolution', onselect: onresolve, disabled: busy }] : [])
	]);
	const note = $derived(freshNote(live.recent, s.key, session.me?.email));
	// The readiness word is the server's answer rendered short (kb/ADR-0018 R1); a done card shows its resolution.
	const status = $derived(storyStatus(item));
	// Urgent: the server's sources are non-empty (a done story has none). The ring is an inset shadow, distinct from
	// `.fresh`'s accent border, and the chip keeps the word so it is never colour alone.
	const urgent = $derived(s.urgent_sources.length > 0);
	/** Tooltip text; `now` is read at each evaluation, so the expiry never freezes. */
	function urgentTip(): string {
		return urgentLine(s.urgent_mark, s.urgent_sources, s.key, new Date(), (r) => (r.item === 'epic' && r.key === s.epic ? epicTitle : undefined));
	}
</script>

<article
	class="card"
	class:fresh={note}
	class:done={s.delivery === 'done'}
	class:urgent={urgent}
	data-testid="card-{s.key}"
	tabindex="-1"
	draggable="true"
	{ondragstart}
	{ondragend}
>
	<div class="head">
		<a class="title" data-sveltekit-noscroll {href}>{s.title}</a>
		{#if items.length}<ActionsMenu label="Actions for {s.key}" {items} control="menu" />{/if}
	</div>
	<div class="meta">
		<span class="key">{s.key}</span>
		{#if urgent}<UrgentChip line={urgentTip()} />{/if}
		{#if s.kind !== 'story'}<span class="kind">{s.kind === 'bug' ? 'Bug' : 'Chore'}</span>{/if}
		{#if epicTitle}<span class="epic" title={s.epic ?? undefined}>{epicTitle}</span>{/if}
		<!-- The first goal the server says the story serves (kb/ADR-0014 R3); the drawer lists them all. -->
		{#if s.goals[0]}<span class="goal" title={s.goals.map((g) => g.outcome).join('\n')}>goal: {truncate(s.goals[0].outcome, 40)}</span>{/if}
	</div>
	<div class="foot">
		<span title={item.readiness.ready ? undefined : item.readiness.reasons.map((r) => fullLabel(r)).join('\n')}
			><StatusDot tone={status.tone} label={status.word} /></span
		>
		<span class="end">
			{#if pr.status && prLink}
				{#if prHref}
					<a
						class="pr {pr.status}"
						data-testid="pr-badge"
						href={prHref}
						target="_blank"
						rel="noopener noreferrer"
						aria-label={prAria}
						title={s.prs.map(prLine).join('\n')}
					>
						{@render prIcon()}
						<span class="visually-hidden">{pr.status}</span>
					</a>
				{:else}
					<span class="pr {pr.status}" data-testid="pr-badge" role="img" aria-label={prAria} title={s.prs.map(prLine).join('\n')}
						>{@render prIcon()}</span
					>
				{/if}
			{/if}
			<Avatar email={s.owner} />
		</span>
	</div>
	{#if note}<p class="note">{note}</p>{/if}
</article>

{#snippet prIcon()}
	<svg
		width="15"
		height="15"
		viewBox="0 0 24 24"
		fill="none"
		stroke="currentColor"
		stroke-width="2"
		stroke-linecap="round"
		aria-hidden="true"
		><circle cx="6" cy="6" r="2.5" /><circle cx="6" cy="18" r="2.5" /><circle cx="18" cy="18" r="2.5" /><path
			d="M6 8.5v7M18 15.5V9a3 3 0 0 0-3-3h-4"
		/></svg
	>
{/snippet}

<style>
	.card {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 4px;
		padding: 8px 10px;
		border: 1px solid var(--line-control);
		border-radius: 6px;
		background: var(--raised);
		cursor: grab;
	}
	.card.urgent {
		box-shadow: inset 0 0 0 2px var(--danger);
	}
	/* Both rings at once: the urgent inset ring and app.css's live-change ring outside the border. */
	.card.urgent.fresh {
		box-shadow: inset 0 0 0 2px var(--danger), 0 0 0 3px var(--selection);
	}
	.head {
		display: flex;
		align-items: flex-start;
		gap: 6px;
	}
	.title {
		flex: 1;
		min-width: 0;
		overflow-wrap: anywhere;
		color: var(--foreground);
		font-size: 15px;
		font-weight: 500;
		line-height: 1.35;
		text-decoration: none;
	}
	.title:hover {
		text-decoration: underline;
	}
	/* A done card reads quieter through its title colour, not opacity: --muted on --raised passes 4.5:1
	   (tokens.contrast.test.ts), where opacity would take every text below it. */
	.done .title {
		color: var(--muted);
	}
	.meta {
		display: flex;
		flex-wrap: wrap;
		gap: 2px 10px;
		margin-top: -2px;
		color: var(--muted);
		font-size: 13px;
	}
	.kind {
		color: var(--foreground);
		font-weight: 600;
	}
	.epic,
	.goal {
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.foot {
		display: flex;
		align-items: center;
		gap: 10px;
		color: var(--muted);
		font-size: 13px;
	}
	.end {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-left: auto;
	}
	.pr {
		display: grid;
		color: var(--muted);
	}
	.pr.open {
		color: var(--pr-open);
	}
	.pr.merged {
		color: var(--pr-merged);
	}
</style>
