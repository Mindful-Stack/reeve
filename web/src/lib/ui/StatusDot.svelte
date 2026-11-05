<!-- web/src/lib/ui/StatusDot.svelte: status as a dot plus a word (never colour alone). -->
<script lang="ts">
	let { tone, label }: { tone: 'ready' | 'warn' | 'danger' | 'muted'; label: string } = $props();
</script>

<span class="status-dot"><span class="dot {tone}" aria-hidden="true"></span>{label}</span>

<style>
	.status-dot {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
		color: var(--foreground);
	}
	.dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--muted);
	}
	.ready {
		background: var(--ready);
	}
	.warn {
		background: var(--warn);
	}
	.danger {
		background: var(--danger);
	}
</style>
