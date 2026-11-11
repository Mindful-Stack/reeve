<!-- web/src/lib/components/DecisionBanner.svelte: the board's one line about decisions that need attention, each part a link into the decisions page. -->
<script lang="ts">
	import type { DecisionQueue } from '#lib/api/types.ts';

	let { summary }: { summary: DecisionQueue['summary'] } = $props();

	// A part with a zero count is hidden; the count picks the singular or plural form.
	const parts = $derived(
		[
			{ n: summary.review, href: '/decisions#review', text: (n: number) => `${n} ${n === 1 ? 'record' : 'records'} to review` },
			{ n: summary.this_sprint, href: '/decisions#this-sprint', text: (n: number) => `${n} ${n === 1 ? 'decision' : 'decisions'} needed this sprint` },
			{ n: summary.next_sprint, href: '/decisions#next-sprint', text: (n: number) => `${n} ${n === 1 ? 'decision' : 'decisions'} needed next sprint` }
		].filter((p) => p.n > 0)
	);
</script>

{#if parts.length}
	<nav class="decision-banner" aria-label="Decisions needing attention">
		<span aria-hidden="true">⚑</span>
		{#each parts as p, i (p.href)}
			{#if i > 0}<span aria-hidden="true">{' · '}</span>{/if}<a href={p.href}>{p.text(p.n)}</a>
		{/each}
	</nav>
{/if}

<style>
	.decision-banner {
		margin: 0 0 12px;
		padding: 6px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--warn-soft);
	}
</style>
