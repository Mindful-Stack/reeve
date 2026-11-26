<script lang="ts">
	import TemplatePicker from './TemplatePicker.svelte';
	import type { BriefTemplateLevel } from '#lib/api/types.ts';
	import { NOTHING_APPLIED, type Applied } from '#lib/templates.ts';
	let {
		level,
		start = '',
		prefill = true,
		kept = NOTHING_APPLIED
	}: { level: BriefTemplateLevel; start?: string; prefill?: boolean; kept?: Applied } = $props();
	// svelte-ignore state_referenced_locally
	let text = $state(start);
	// What the picker last applied, as a form keeps it with its draft across a reopen.
	// svelte-ignore state_referenced_locally
	let last = $state(kept);
	let ready = $state(false);
</script>

<TemplatePicker {level} {prefill} bind:text bind:ready bind:last />
<textarea aria-label="Brief" bind:value={text}></textarea>
<output aria-label="Templates">{ready ? 'ready' : 'loading'}</output>
