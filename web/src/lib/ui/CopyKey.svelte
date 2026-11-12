<!-- web/src/lib/ui/CopyKey.svelte: a small button that copies an item key. It lives inside draggable cards, so it never
     starts a drag and its click never reaches the card. The outcome is announced in a visually hidden polite status (it
     must not shift the card's meta row) and shown by the glyph and the title. -->
<script lang="ts">
	let { key }: { key: string } = $props();
	let said = $state('');
	let timer: ReturnType<typeof setTimeout> | undefined;

	async function copy(e: MouseEvent) {
		e.preventDefault();
		e.stopPropagation();
		clearTimeout(timer);
		try {
			// navigator.clipboard is undefined over plain http outside localhost; the TypeError lands here too.
			await navigator.clipboard.writeText(key);
			said = 'Copied';
		} catch {
			said = 'Copy failed';
		}
		timer = setTimeout(() => (said = ''), 1500);
	}
	$effect(() => () => clearTimeout(timer));
</script>

<button
	type="button"
	class="ghost icon copy"
	aria-label="Copy {key}"
	title={said || `Copy ${key}`}
	draggable="false"
	ondragstart={(e) => {
		e.preventDefault();
		e.stopPropagation();
	}}
	onpointerdown={(e) => e.stopPropagation()}
	onclickcapture={copy}
	><svg viewBox="0 0 16 16" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
		{#if said === 'Copied'}
			<path d="M3 8.5l3.5 3.5L13 4.5" />
		{:else}
			<rect x="5.5" y="5.5" width="8" height="8" rx="1.5" />
			<path d="M10.5 5.5v-2a1.5 1.5 0 0 0-1.5-1.5h-5A1.5 1.5 0 0 0 2.5 3.5v5A1.5 1.5 0 0 0 4 10h1.5" />
		{/if}
	</svg></button
><span class="visually-hidden copy-said" role="status">{said}</span>

<style>
	.copy {
		padding: 0 2px;
		line-height: 1;
		color: var(--muted);
	}
	.copy:hover {
		color: var(--foreground);
	}
</style>
