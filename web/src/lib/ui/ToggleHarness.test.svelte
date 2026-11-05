<!-- Test harness: Toggle in tabs mode with a real tabpanel. -->
<script lang="ts">
	import Toggle from './Toggle.svelte';

	let { options }: { options: { value: string; label: string }[] } = $props();
	let value = $state('a');
	let selectedTabId = $state<string | undefined>();
</script>

<Toggle label="T" mode="tabs" {options} {value} onchange={(v) => void (value = v)} panel="pan" bind:selectedTabId />
<div role="tabpanel" id="pan" aria-labelledby={selectedTabId}>{value}</div>
