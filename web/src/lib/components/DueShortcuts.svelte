<!-- web/src/lib/components/DueShortcuts.svelte: sprint-end shortcuts under a decision's Due date (GRA-636). It reads
     GET /sprints once when it mounts (it lives as long as the form it sits in) and fills the date only: a decision keeps
     no sprint (kb/ADR-0014 R1, R6), and Due stays optional and editable by hand (kb/ADR-0004 R2). Until the sprints
     answer, when the read fails and when no end is today or later, it renders nothing: the date field works without it.
     Place it after the Due field's </label>, so its buttons stay out of the field's name. -->
<script lang="ts">
	import { onMount } from 'svelte';
	import { api, unwrap } from '#lib/api/client.ts';
	import { sprintEnds, type SprintEnd } from '#lib/sprint.ts';

	let { value = $bindable(''), disabled = false }: { value?: string; disabled?: boolean } = $props();

	let shortcuts = $state<SprintEnd[]>([]);
	onMount(() => {
		unwrap(api.GET('/api/v1/sprints')).then(
			(sp) => (shortcuts = sprintEnds(sp)),
			() => {} // no shortcuts; the date field is enough
		);
	});
</script>

{#if shortcuts.length}
	<div class="shortcuts" role="group" aria-label="Due date shortcuts">
		{#each shortcuts as sc (sc.shortcut)}
			<!-- Pressed while it is the date in the field: the visible and announced confirmation of a click. -->
			<button
				type="button"
				aria-pressed={sc.ends === value}
				{disabled}
				onclick={(e) => {
					value = sc.ends;
					// A script write fires no event: say it as the date field would, so lib/ui/Dialog.svelte counts it as an edit.
					e.currentTarget.dispatchEvent(new Event('change', { bubbles: true }));
				}}>{sc.label}</button
			>
		{/each}
	</div>
{/if}

<style>
	.shortcuts {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		min-width: 0;
	}
</style>
