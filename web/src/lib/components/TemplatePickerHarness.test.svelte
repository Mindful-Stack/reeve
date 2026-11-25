<script lang="ts">
	import TemplatePicker from './TemplatePicker.svelte';
	import type { BriefTemplateLevel } from '#lib/api/types.ts';
	let { level, start = '', prefill = true }: { level: BriefTemplateLevel; start?: string; prefill?: boolean } = $props();
	// svelte-ignore state_referenced_locally
	let text = $state(start);
	let ready = $state(false);
</script>

<TemplatePicker {level} {prefill} bind:text bind:ready />
<textarea aria-label="Brief" bind:value={text}></textarea>
<output aria-label="Templates">{ready ? 'ready' : 'loading'}</output>
