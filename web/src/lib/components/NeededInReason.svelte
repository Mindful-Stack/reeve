<!-- web/src/lib/components/NeededInReason.svelte: why a decision is needed, as a phrase; the item it names (in discovery, or holding a promise) is a link: an initiative or epic page, or a story's drawer over this view. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { NeededInReason } from '#lib/api/types.ts';
	import { neededInText, reasonHref, reasonParts } from '#lib/decisions.ts';

	let { reason }: { reason: NeededInReason } = $props();
	const parts = $derived(reasonParts(reason));
	const href = $derived(reasonHref(reason, page.url));
	// A story opens its drawer over this view, which must not move (drawer.ts: storyHref); `reasonHref` routes by the
	// same level, so the link and the attribute share one condition.
	const drawer = $derived((reason.kind === 'discovery' || reason.kind === 'promise') && reason.level === 'story');
</script>

{#if parts && href}{parts.before}<a class="key" data-field="{reason.kind}-item" data-sveltekit-reset={drawer ? 'false' : undefined} {href}>{parts.key}</a>{parts.after}{:else}{neededInText(reason)}{/if}
