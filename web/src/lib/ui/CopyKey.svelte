<!-- web/src/lib/ui/CopyKey.svelte: a small button that copies an item key. It lives inside draggable cards, so it never
     starts the card's drag and its click never reaches the card. The outcome is announced in a visually hidden polite status (it
     must not shift the card's meta row) and shown by the icon and the title. -->
<script lang="ts">
	let { key }: { key: string } = $props();
	let said = $state('');
	let timer: ReturnType<typeof setTimeout> | undefined;
	/** Counts clicks (and unmounts): only the latest one may set the text or schedule the reset. */
	let seq = 0;

	async function copy(e: MouseEvent) {
		e.preventDefault();
		e.stopPropagation();
		clearTimeout(timer);
		const n = ++seq;
		said = ''; // so a repeat copy changes the live region's text and is announced again
		let result: string;
		try {
			// navigator.clipboard is undefined over plain http outside localhost; the TypeError lands here too.
			await navigator.clipboard.writeText(key);
			result = 'Copied';
		} catch {
			result = 'Copy failed';
		}
		if (n !== seq) return;
		said = result;
		timer = setTimeout(() => (said = ''), 1500);
	}
	$effect(() => () => {
		seq++;
		clearTimeout(timer);
	});
</script>

<!-- Pressing a non-draggable descendant of a draggable card starts the card's drag (dragstart fires on the card, never
     on the button), so the mouse press is cancelled (mousedown, not pointerdown: cancelling pointerdown suppresses mousedown, which page menus close on); a click, Enter and Space still work. -->
<button
	type="button"
	class="ghost icon copy"
	aria-label="Copy {key}"
	title={said || `Copy ${key}`}
	onmousedown={(e) => e.preventDefault()}
	onclickcapture={copy}
	><svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
		{#if said === 'Copied'}
			<path d="M3 8.5l3.5 3.5L13 4.5" />
		{:else if said === 'Copy failed'}
			<path d="M4 4l8 8M12 4l-8 8" />
		{:else}
			<rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
			<path d="M10.5 5.5v-2a1.5 1.5 0 0 0-1.5-1.5h-5A1.5 1.5 0 0 0 2.5 3.5v5A1.5 1.5 0 0 0 4 10h1.5" />
		{/if}
	</svg></button
><span class="visually-hidden" role="status">{said}</span>

<style>
	/* The global button box is 32px; keep a 24px target without growing the card's meta row. */
	.copy {
		width: 24px;
		height: 24px;
		margin-block: -3px;
		align-self: center;
		padding: 0;
		line-height: 1;
		color: var(--muted);
	}
	.copy:hover {
		color: var(--foreground);
	}
</style>
