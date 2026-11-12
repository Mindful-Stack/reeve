<!-- A promise chip (kb/ADR-0025): the words carry the state, period and date; tone and icon only repeat them, so nothing
     is colour alone and nothing lives only in a tooltip. Text is --foreground on the tone's soft background. -->
<script lang="ts">
	import type { EpicPromise, StoryPromise } from '#lib/api/types.ts';
	import { chip } from '#lib/promise.ts';

	let { promise }: { promise: StoryPromise | EpicPromise } = $props();

	const c = $derived(chip(promise));
</script>

<span class="promise-chip {c.tone}"
	>{#if c.icon === 'clock'}<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" aria-hidden="true"
			><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 3" /></svg
		>{:else if c.icon === 'check'}<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" aria-hidden="true"
			><path d="M4 12l5 5L20 6" /></svg
		>{:else if c.icon === 'calendar'}<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" aria-hidden="true"
			><rect x="3" y="5" width="18" height="16" rx="2" /><path d="M3 10h18M8 3v4M16 3v4" /></svg
		>{/if}{c.text}</span
>

<style>
	.promise-chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		color: var(--foreground);
		border-radius: 999px;
		padding: 0 6px;
		font-size: 12px;
		white-space: nowrap;
	}
	/* Each pair (--foreground on the background) is pinned at AA in tokens.contrast.test.ts. */
	.warn {
		background: var(--warn-soft);
	}
	.warn svg {
		color: var(--warn);
	}
	.ready {
		background: var(--ready-soft);
	}
	.ready svg {
		color: var(--ready);
	}
	.muted {
		background: var(--lighter-background);
	}
	.neutral {
		background: var(--selection);
	}
</style>
