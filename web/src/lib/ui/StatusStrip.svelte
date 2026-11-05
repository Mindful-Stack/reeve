<!-- web/src/lib/ui/StatusStrip.svelte: the detail pages' status strip: discovery, what blocks, progress. -->
<script lang="ts">
	import type { Cell } from '#lib/strip.ts';

	let {
		discovery,
		cell,
		progress,
		onmarkdiscovery
	}: {
		/** `at` is shown as given, so callers pass an already formatted date. */
		discovery: { done: boolean; by?: string; at?: string };
		cell: Cell;
		progress: { done: number; total: number };
		onmarkdiscovery?: () => void;
	} = $props();

	const doneText = $derived(
		[
			'Complete',
			[discovery.by ? `by ${discovery.by}` : '', discovery.at ?? ''].filter(Boolean).join(', ')
		]
			.filter(Boolean)
			.join(discovery.by ? ' ' : ', ')
	);
	const pct = $derived(progress.total ? Math.round((progress.done / progress.total) * 100) : 0);
</script>

<div role="group" aria-label="Status" class="strip">
	<div class="cell">
		<span class="label">Discovery</span>
		{#if discovery.done}
			<span>{doneText}</span>
		{:else}
			<span>Open</span>
			{#if onmarkdiscovery}<button type="button" onclick={onmarkdiscovery}>Mark discovery complete</button>{/if}
		{/if}
	</div>
	{#if cell.blocking && cell.anchor}
		<a class="cell blocking" href={cell.anchor}>
			<span class="label">Blockers</span>
			<span>{cell.text}</span>
			{#if cell.detail}<span class="detail">{cell.detail}</span>{/if}
		</a>
	{:else}
		<div class="cell">
			<span class="label">Blockers</span>
			<span>{cell.text}</span>
			{#if cell.detail}<span class="detail">{cell.detail}</span>{/if}
		</div>
	{/if}
	<div class="cell">
		<span class="label">Progress</span>
		{#if progress.total === 0}
			<span>No stories yet</span>
		{:else}
			<span>{progress.done} of {progress.total} stories done</span>
			<div
				class="bar"
				role="progressbar"
				aria-label="Stories done"
				aria-valuetext="{progress.done} of {progress.total} stories done"
				aria-valuemin="0"
				aria-valuemax={progress.total}
				aria-valuenow={progress.done}
			>
				<div class="fill" style:width="{pct}%"></div>
			</div>
		{/if}
	</div>
</div>

<style>
	.strip {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
		border: 1px solid var(--line);
		border-radius: 6px;
		gap: 1px;
		background: var(--raised);
		overflow: hidden;
	}
	.cell {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 4px;
		padding: 10px 14px;
		color: inherit;
		text-decoration: none;
		background: var(--raised);
		box-shadow: 0 0 0 1px var(--line);
	}
	.label {
		font-size: 13px;
		text-transform: uppercase;
		letter-spacing: 0.04em;
	}
	.detail {
		font-size: 14px;
	}
	.blocking {
		background: var(--warn-soft);
	}
	.bar {
		align-self: stretch;
		height: 6px;
		border-radius: 3px;
		background: var(--line);
		overflow: hidden;
	}
	.fill {
		height: 100%;
		background: var(--ready);
	}
</style>
