<script lang="ts">
	import type { Initiative } from '#lib/api/types.ts';
	import GoalLinkButton from './GoalLinkButton.svelte';
	import { freshNote, live } from '#lib/events.svelte.ts';
	import { age, month, short } from '#lib/format.ts';
	import { session } from '#lib/me.svelte.ts';
	import type { OpenDecisions } from '#lib/planning.ts';
	import ActionsMenu, { type MenuItem } from '#lib/ui/ActionsMenu.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';

	let {
		initiative,
		decisions,
		buckets = [],
		busy = false,
		onmove
	}: {
		initiative: Initiative;
		/** The initiative's blocking decisions (`blockingByItem`), or undefined when none blocks it. */
		decisions?: OpenDecisions;
		/** The configured horizons; the menu offers "Move to" each of them but the current one. */
		buckets?: string[];
		/** Disables the "Move to" items while a write is in flight. */
		busy?: boolean;
		onmove?: (key: string, horizon: string) => void;
	} = $props();

	const id = $props.id();
	const mark = $derived(initiative.discovery_complete);
	const progress = $derived(initiative.progress ?? null);
	const pct = $derived(progress === null ? 0 : Math.round(progress * 100));
	const note = $derived(freshNote(live.recent, initiative.key, session.me?.email));
	const items = $derived<MenuItem[]>(
		onmove
			? buckets
					.filter((b) => b !== initiative.horizon)
					.map((b) => ({ label: `Move to ${b}`, onselect: () => onmove(initiative.key, b), disabled: busy }))
			: []
	);

	function ondragstart(e: DragEvent) {
		if (!e.dataTransfer) return;
		e.dataTransfer.setData('text/plain', initiative.key);
		e.dataTransfer.effectAllowed = 'move';
	}
</script>

<article
	class="card"
	class:fresh={note}
	data-testid="card-{initiative.key}"
	tabindex="-1"
	draggable="true"
	{ondragstart}
	aria-label={initiative.title}
>
	<div class="head">
		<a class="title" href="/initiatives/{encodeURIComponent(initiative.key)}">{initiative.title}</a>
		{#if items.length}<ActionsMenu label="Actions for {initiative.key}" {items} control="menu" />{/if}
	</div>
	<div class="meta">
		<span class="key">{initiative.key}</span>
		{#if initiative.owner}<span>{short(initiative.owner)}</span>{/if}
		{#if initiative.target_month}<span>{month(initiative.target_month)}</span>{/if}
	</div>
	<div class="line">
		{#if mark}
			<span class="done" title="by {mark.by}, {new Date(mark.at).toLocaleString()}"
				><svg
					width="14"
					height="14"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
					stroke-width="2.5"
					stroke-linecap="round"
					stroke-linejoin="round"
					aria-hidden="true"><path d="M20 6 9 17l-5-5" /></svg
				>Discovery done</span
			>
		{:else}
			<StatusDot tone="warn" label="Discovery open" />
		{/if}
	</div>
	{#if decisions}
		<p class="line blocking" title={decisions.open.map((d) => d.title).join('\n')} aria-describedby="{id}-blocking">
			{decisions.count}
			{decisions.count === 1 ? 'blocking decision' : 'blocking decisions'} · oldest {age(decisions.oldest)}
		</p>
		<ul class="visually-hidden" id="{id}-blocking">
			{#each decisions.open as d (d.id)}<li>{d.title}</li>{/each}
		</ul>
	{/if}
	{#if progress !== null}
		<!-- The server sends a fraction only (InitiativeDto.progress), no story counts, so the label is a percentage. -->
		<div class="progress">
			<div class="bar" aria-hidden="true"><span style="width: {pct}%"></span></div>
			<span>{pct}% done</span>
		</div>
	{/if}
	{#if note}<p class="note">{note}</p>{/if}
	<GoalLinkButton key={initiative.key} />
</article>

<style>
	.card {
		display: grid;
		gap: 6px;
		padding: 10px 12px;
		border: 1px solid var(--line-control);
		border-radius: 6px;
		background: var(--raised);
		cursor: grab;
	}
	.head {
		display: flex;
		align-items: flex-start;
		gap: 6px;
	}
	.title {
		flex: 1;
		color: var(--foreground);
		font-size: 18px;
		font-weight: 600;
		line-height: 1.3;
		text-decoration: none;
	}
	.title:hover {
		text-decoration: underline;
	}
	.meta {
		display: flex;
		flex-wrap: wrap;
		gap: 2px 10px;
		color: var(--muted);
		font-size: 13px;
	}
	.line {
		margin: 0;
		font-size: 13px;
	}
	.done {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	.done svg {
		color: var(--ready);
	}
	.blocking {
		color: var(--foreground);
	}
	.progress {
		display: flex;
		align-items: center;
		gap: 8px;
		color: var(--muted);
		font-size: 13px;
	}
	.bar {
		flex: 1;
		height: 6px;
		border-radius: 3px;
		background: var(--line);
		overflow: hidden;
	}
	.bar span {
		display: block;
		height: 100%;
		background: var(--ready);
	}
</style>
