<!-- web/src/lib/components/NeededInReason.svelte: why a decision is needed, as a phrase; the item in discovery is a link to its page. -->
<script lang="ts">
	import type { NeededInReason } from '#lib/api/types.ts';
	import { discoveryHref } from '#lib/decisions.ts';

	let { reason }: { reason: NeededInReason } = $props();
	const href = $derived(discoveryHref(reason));
</script>

{#if reason.kind === 'goal'}for goal: {reason.outcome}{:else if href}<a class="key" data-field="discovery-item" {href}>{reason.key}</a> is in discovery{:else}{reason.key} is in discovery{/if}
