<!-- web/src/lib/components/NeededInReason.svelte: why a decision or discovery row is needed, as a phrase read off its queue row (reasonLine). The item or decision it names is a link: an initiative or epic page, or a story's or decision's drawer over this view. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { NeededInReason } from '#lib/api/types.ts';
	import { reasonHref, reasonLine, type QueueRow } from '#lib/decisions.ts';

	let { reason, row }: { reason: NeededInReason; row: QueueRow } = $props();
	const parts = $derived(reasonLine(reason, row));
	const href = $derived(reasonHref(reason, page.url));
	// A drawer opens over this view, which must not move (drawer.ts): a story's, or the decision a Now line names.
	const drawer = $derived(('level' in reason && reason.level === 'story') || reason.kind === 'decision_due');
</script>

{parts.before}{#if parts.key && href}<a class="key" data-field="{reason.kind}-item" data-sveltekit-reset={drawer ? 'false' : undefined} {href}>{parts.key}</a>{:else}{parts.key}{/if}{parts.after}
