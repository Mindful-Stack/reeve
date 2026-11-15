<!-- web/src/lib/components/DecisionBanner.svelte: the board's one line about the readiness queue's window lanes: what is in discovery and which decisions are needed this sprint and next, each part a link into the Readiness page. -->
<script lang="ts">
	import type { DecisionQueue } from '#lib/api/types.ts';
	import { plural } from '#lib/format.ts';

	let { queue }: { queue: Pick<DecisionQueue, 'summary' | 'discoveries'> } = $props();

	// One part per window: discovery rows counted from `discoveries` by lane, decisions from the server's summary (nested
	// ones included). A zero part is left out, and within a part a zero kind.
	const parts = $derived(
		(
			[
				['this_sprint', 'this sprint', '/decisions#this-sprint', queue.summary.this_sprint],
				['next_sprint', 'next sprint', '/decisions#next-sprint', queue.summary.next_sprint]
			] as const
		)
			.map(([lane, when, href, decided]) => {
				const found = queue.discoveries.filter((d) => d.lane === lane).length;
				const kinds = [found ? `${found} in discovery` : '', decided ? plural(decided, 'decision', 'decisions') : ''].filter(Boolean);
				return { href, n: found + decided, text: `${kinds.join(' and ')} needed ${when}` };
			})
			.filter((p) => p.n > 0)
	);
</script>

{#if parts.length}
	<nav class="decision-banner" aria-label="Readiness needing attention">
		<span aria-hidden="true">⚑</span>
		{#each parts as p, i (p.href)}
			{#if i > 0}<span aria-hidden="true">{' · '}</span>{/if}<a href={p.href}>{p.text}</a>
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
