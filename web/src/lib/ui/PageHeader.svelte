<!-- web/src/lib/ui/PageHeader.svelte: the page title with a count and its actions. -->
<script lang="ts">
	import type { Snippet } from 'svelte';

	let { title, count, children }: { title: string; count?: number; children?: Snippet } = $props();
</script>

<div class="page-header">
	<h1>{title}{#if count !== undefined}{' '}<span class="count">{count}</span>{/if}</h1>
	{#if children}<div class="actions">{@render children()}</div>{/if}
</div>

<style>
	.page-header {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
		margin-bottom: 14px;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
		/* Below about 18rem beside the title, the actions take their own row instead of a narrow column. */
		flex: 1 1 18rem;
		font-size: 14px;
	}
</style>
