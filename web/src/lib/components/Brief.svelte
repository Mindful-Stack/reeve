<script lang="ts">
	import { renderMarkdown } from '#lib/markdown.ts';

	let { markdown }: { markdown: string } = $props();

	const html = $derived(markdown.trim() ? renderMarkdown(markdown) : '');
</script>

{#if html}
	<!-- Sanitised by renderMarkdown (#lib/markdown.ts). -->
	<div class="brief">{@html html}</div>
{:else}
	<p class="empty">No brief yet.</p>
{/if}

<style>
	.brief {
		line-height: 1.5;
		overflow-wrap: anywhere;
	}
	.brief :global(:first-child) {
		margin-top: 0;
	}
	.brief :global(pre) {
		overflow-x: auto;
		padding: 0.5rem;
		background: var(--lighter-background);
		border-radius: 4px;
	}
	.brief :global(code) {
		font-family: var(--mono);
	}
	.empty {
		margin: 0;
		color: var(--muted);
	}
</style>
