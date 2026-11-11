<!-- web/src/lib/components/StoryRow.svelte: one compact story row on Home: the key and title (opening the story drawer over the
     page), the urgent and goal chips, the delivery state and the epic key, and, when Home offers it, a Start button. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { StoryWithReadiness } from '#lib/api/types.ts';
	import { COLUMN_LABEL } from '#lib/board.ts';
	import { storyHref } from '#lib/drawer.ts';
	import { urgentLine } from '#lib/urgency.ts';
	import GoalChip from './GoalChip.svelte';
	import UrgentChip from './UrgentChip.svelte';

	let {
		row,
		onstart,
		busy = false
	}: {
		row: StoryWithReadiness;
		/** Shows a Start button that calls this; absent, the row has no button. */
		onstart?: (row: StoryWithReadiness) => void;
		/** Disables Start while a write is in flight. */
		busy?: boolean;
	} = $props();

	const s = $derived(row.story);
</script>

<li class="row" data-key={s.key}>
	<span class="key">{s.key}</span>
	<span class="main">
		<a class="title" data-sveltekit-reset="false" href={storyHref(page.url, s.key)}>{s.title}</a>
		<span class="meta">
			{#if s.urgent_sources.length > 0}<UrgentChip line={urgentLine(s.urgent_mark, s.urgent_sources, s.key, new Date(), () => undefined)} />{/if}
			<GoalChip goals={s.goals} />
			<span>{COLUMN_LABEL[s.delivery]}</span>
			{#if s.epic}<span>{s.epic}</span>{/if}
		</span>
	</span>
	{#if onstart}
		<button type="button" class="primary start" disabled={busy} aria-label="Start {s.key}" onclick={() => onstart(row)}>Start</button>
	{/if}
</li>

<style>
	.row {
		display: grid;
		grid-template-columns: auto minmax(0, 1fr) auto;
		align-items: center;
		gap: 2px 12px;
		padding: 8px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
		list-style: none;
	}
	.key {
		font-family: var(--mono);
		font-size: 12px;
		color: var(--muted);
		white-space: nowrap;
	}
	.main {
		display: grid;
		gap: 2px;
		min-width: 0;
	}
	.title {
		color: var(--foreground);
		font-weight: 500;
		text-decoration: none;
		overflow-wrap: anywhere;
	}
	.title:hover {
		text-decoration: underline;
	}
	.meta {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 2px 10px;
		color: var(--muted);
		font-size: 13px;
	}
	@media (max-width: 600px) {
		.row {
			grid-template-columns: auto minmax(0, 1fr);
		}
		.start {
			grid-column: 1 / -1;
			width: 100%;
		}
	}
</style>
