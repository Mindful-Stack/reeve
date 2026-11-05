<!-- web/src/lib/ui/Notice.svelte: one notice style for info and errors, with optional Retry and Dismiss. -->
<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		tone = 'info',
		onretry,
		ondismiss,
		children
	}: { tone?: 'info' | 'error'; onretry?: () => void; ondismiss?: () => void; children: Snippet } = $props();
</script>

<div class="notice {tone}" role={tone === 'error' ? 'alert' : 'status'}>
	<span class="text">{@render children()}</span>
	{#if onretry}<button type="button" onclick={onretry}>Retry</button>{/if}
	{#if ondismiss}<button type="button" class="ghost" onclick={ondismiss}>Dismiss</button>{/if}
</div>

<style>
	.notice {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
		margin: 0 0 12px;
		padding: 8px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.text {
		flex: 1 1 240px;
	}
	.error {
		border-color: var(--danger);
	}
</style>
