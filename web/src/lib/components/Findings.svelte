<script lang="ts">
	import type { Finding } from '#lib/api/types.ts';
	import { findingLabel } from '#lib/findings.ts';

	let { findings }: { findings: Finding[] } = $props();
</script>

{#if findings.length > 0}
	<section aria-labelledby="findings" class="findings">
		<h2 id="findings">Findings</h2>
		<p class="muted">Worth a look; findings never block a move.</p>
		<ul>
			{#each findings as f, n (n)}
				<li>{findingLabel(f)}</li>
			{/each}
		</ul>
	</section>
{/if}

<style>
	.findings ul {
		margin: 0.25rem 0 0;
		padding-left: 1.25rem;
	}
	.muted {
		color: var(--muted);
		font-size: 14px;
		margin: 0;
	}
</style>
