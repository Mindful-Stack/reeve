<!-- A promise chip (kb/ADR-0025): the words carry the state, period and date; tone and icon only repeat them, so nothing
     is colour alone and nothing lives only in a tooltip. Text is --foreground on the tone's soft background. -->
<script lang="ts">
	import type { EpicPromise, StoryPromise } from '#lib/api/types.ts';
	import { chip } from '#lib/promise.ts';

	let {
		promise,
		via = true
	}: {
		promise: StoryPromise | EpicPromise;
		/** Name the epic when its promise sets a story's date; false on the epic's own page. */
		via?: boolean;
	} = $props();

	const c = $derived(chip(promise, { via }));
</script>

<span class="promise-chip tone-{c.tone}"
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
		/* A card is narrower than the longest text: the chip wraps inside it rather than overflowing, and never cuts the
		   state off with an ellipsis. */
		max-width: 100%;
		white-space: normal;
		border-radius: 10px;
		padding: 0 6px;
		font-size: 12px;
	}
	.promise-chip svg {
		flex-shrink: 0;
	}
	/* Each pair (--foreground on the background) is pinned at AA in tokens.contrast.test.ts. */
	.tone-warn {
		background: var(--warn-soft);
	}
	.tone-warn svg {
		color: var(--warn);
	}
	.tone-ready {
		background: var(--ready-soft);
	}
	.tone-ready svg {
		color: var(--ready);
	}
	.tone-muted {
		background: var(--lighter-background);
	}
	.tone-neutral {
		background: var(--selection);
	}
</style>
