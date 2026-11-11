<!-- Passes `story.key` the way the drawer does, so a refetch hands a new object with the same key. -->
<script lang="ts">
	import Attachments from './Attachments.svelte';
	let { story }: { story: { key: string } } = $props();
</script>

<section><h3>Description</h3><Attachments itemKey={story.key} /></section>
