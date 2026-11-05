<!-- web/src/lib/ui/Dialog.svelte: a modal dialog over <dialog>. Escape reports onclose; the parent unmounts it.
	`closable={false}` (e.g. while a create is pending) disables × and swallows Escape without reporting onclose; a
	browser that closes the dialog anyway (one that ignores a prevented cancel) gets it shown again. -->
<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		onclose,
		closable = true,
		children
	}: { title: string; onclose: () => void; closable?: boolean; children: Snippet } = $props();

	const id = $props.id();
	let el: HTMLDialogElement;

	let closing = false;

	function show() {
		if (typeof el.showModal === 'function') el.showModal();
		else el.setAttribute('open', '');
	}

	function focusFirst() {
		el.querySelector<HTMLElement>('input:not([type=hidden]):not(:disabled), select, textarea, button:not(.close)')?.focus();
	}

	$effect(() => {
		const opener = document.activeElement as HTMLElement | null;
		closing = false;
		show();
		focusFirst();
		return () => {
			closing = true;
			if (typeof el.close === 'function' && el.open) el.close();
			if (opener?.isConnected && (document.activeElement === document.body || el.contains(document.activeElement)))
				opener.focus();
		};
	});
</script>

<dialog
	bind:this={el}
	onclose={() => {
		if (closing) return;
		if (closable) onclose();
		else {
			show();
			focusFirst();
		}
	}}
	aria-labelledby="{id}-title"
	oncancel={(e) => {
		e.preventDefault();
		if (closable) onclose();
	}}
>
	<div class="head">
		<h2 id="{id}-title">{title}</h2>
		<button type="button" class="ghost icon close" aria-label="Close" disabled={!closable} onclick={onclose}>×</button>
	</div>
	{@render children()}
</dialog>

<style>
	.head {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-bottom: 12px;
	}
	.head h2 {
		flex: 1;
	}
</style>
