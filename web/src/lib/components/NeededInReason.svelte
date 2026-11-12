<!-- web/src/lib/components/NeededInReason.svelte: why a decision is needed, as a phrase; the item it names (in discovery, or holding a promise) is a link: an initiative or epic page, or a story's drawer over this view. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { NeededInReason } from '#lib/api/types.ts';
	import { neededInText, reasonHref } from '#lib/decisions.ts';

	let { reason }: { reason: NeededInReason } = $props();
	const text = $derived(neededInText(reason));
	const href = $derived(reasonHref(reason, page.url));
	const key = $derived(reason.kind === 'discovery' || reason.kind === 'promise' ? reason.key : null);
	// The phrase names the key once; it becomes the link and the words around it stay text.
	const at = $derived(key == null ? -1 : text.indexOf(key));
</script>

{#if href && key != null && at >= 0}{text.slice(0, at)}<a class="key" data-field="{reason.kind}-item" {href}>{key}</a>{text.slice(at + key.length)}{:else}{text}{/if}
