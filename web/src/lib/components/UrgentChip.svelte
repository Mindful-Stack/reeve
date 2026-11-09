<!-- The urgent chip: the word and a flag, never colour alone. `line` is the tooltip (see urgentLine in #lib/urgency.ts). -->
<script lang="ts">
	let { line }: { line: string } = $props();
</script>

<span class="urgent-chip" title={line}
	><svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"
		><path d="M5 2h2v20H5zM8 3h12l-3 5 3 5H8z" /></svg
	>Urgent</span
>

<style>
	.urgent-chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		color: var(--danger);
		background: var(--danger-soft);
		border-radius: 999px;
		padding: 0 6px;
		font-size: 12px;
		white-space: nowrap;
	}
</style>
