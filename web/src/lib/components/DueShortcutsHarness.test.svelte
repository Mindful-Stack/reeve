<script lang="ts">
	import Dialog from '#lib/ui/Dialog.svelte';
	import DueShortcuts from './DueShortcuts.svelte';
	let { onclose }: { onclose: () => void } = $props();
</script>

<!-- In Part A's Dialog, as the epic and initiative pages' Add decision forms are. -->
<Dialog title="Add decision" {onclose}><DueShortcuts /></Dialog>
