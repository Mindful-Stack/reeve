<script lang="ts">
	import Dialog from './Dialog.svelte';
	import Picker from './Picker.svelte';
	import type { PickerOption } from '#lib/picker.ts';
	let { options, start = '', empty, onsubmit, onkeydown, onchange, onclose }: {
		options: PickerOption[]; start?: string; empty?: string;
		onsubmit: () => void; onkeydown: (e: KeyboardEvent) => void; onchange: (v: string) => void; onclose: () => void;
	} = $props();
	// svelte-ignore state_referenced_locally
	let value = $state(start);
</script>

<!-- Inside Part A's Dialog, as the backlog's Change epic is: its focusFirst() focuses the picker by script, and its
     backdrop close must see a pick as an edit. -->
<Dialog title="Change epic" {onclose}>
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<form onsubmit={(e) => { e.preventDefault(); onsubmit(); }} {onkeydown}>
		<label>Epic <Picker label="Epic" {options} bind:value {empty} {onchange} /></label>
		<output data-testid="bound">{value}</output>
		<button type="submit">Save</button>
	</form>
</Dialog>
