<!-- web/src/lib/components/EpicCard.svelte: an epic on its initiative's page: title, key, story count, status
     line and progress, from counts the page derives (#lib/initiative.ts). -->
<script lang="ts">
	import type { Epic } from '#lib/api/types.ts';
	import GoalLinkButton from './GoalLinkButton.svelte';
	import Glyph from '#lib/components/Glyph.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';
	import { epicStatus, type EpicCounts } from '#lib/initiative.ts';

	let { epic, counts: c, waiting }: { epic: Pick<Epic, 'key' | 'title'>; counts: EpicCounts; waiting: string | null } = $props();

	const pct = $derived(c.total ? Math.round((c.done / c.total) * 100) : 0);
</script>

<li class="epic-card">
	<a class="epic-title" href="/epics/{encodeURIComponent(epic.key)}">{epic.title}</a>
	<span class="muted line"><span class="key"><Glyph level="epic" />{epic.key}</span> · {c.total} {c.total === 1 ? 'story' : 'stories'}</span>
	<span class="line"
		><StatusDot tone={waiting || c.needCriteria > 0 ? 'warn' : c.total === 0 ? 'muted' : 'ready'} label={epicStatus(c, waiting)} /></span
	>
	{#if c.total > 0}
		<span class="progress">
			<span class="bar" aria-hidden="true"><span style:width="{pct}%"></span></span>
			<span>{c.done} of {c.total} done</span>
		</span>
	{/if}
	<GoalLinkButton key={epic.key} />
</li>

<style>
	.epic-card {
		display: grid;
		align-content: start;
		gap: 2px;
		padding: 10px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.epic-title {
		color: var(--foreground);
		font-size: 16px;
		font-weight: 600;
		text-decoration: none;
		overflow-wrap: anywhere;
	}
	.epic-title:hover {
		text-decoration: underline;
	}
	.epic-card > :global(.goal-link) {
		margin-top: 6px;
	}
	.line {
		font-size: 14px;
	}
	.progress {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-top: 4px;
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
