<script lang="ts">
	import { renderMarkdown } from '#lib/markdown.ts';
	import { rendered } from '#lib/rendered.ts';

	// One comment's body, rendered as Markdown like a story's description: an imported Jira comment carries links and
	// lists, and anything typed here may too.
	let { markdown }: { markdown: string } = $props();

	const html = $derived(renderMarkdown(markdown));
</script>

<!-- Sanitised by renderMarkdown (#lib/markdown.ts). -->
<div class="comment-body" {@attach rendered()}>{@html html}</div>

<style>
	.comment-body :global(img) {
		max-width: 100%;
		height: auto;
		max-height: 30rem;
	}
	.comment-body :global(.image-unavailable) {
		display: inline-block;
		padding: 0.125rem 0.5rem;
		border-radius: 3px;
		background: var(--lighter-background);
		color: var(--muted);
		font-size: 13px;
	}
	.comment-body {
		margin: 0.1rem 0 0;
		overflow-wrap: anywhere;
	}
	.comment-body :global(p),
	.comment-body :global(ul),
	.comment-body :global(ol),
	.comment-body :global(pre),
	.comment-body :global(blockquote) {
		margin: 0.25rem 0 0;
	}
	.comment-body :global(:first-child) {
		margin-top: 0;
	}
	.comment-body :global(pre) {
		overflow-x: auto;
		padding: 0.5rem;
		background: var(--lighter-background);
		border-radius: 4px;
	}
	.comment-body :global(code) {
		font-family: var(--mono);
	}
</style>
