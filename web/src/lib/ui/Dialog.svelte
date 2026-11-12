<!-- web/src/lib/ui/Dialog.svelte: a modal dialog over <dialog>. Escape reports onclose, and so does a click on the backdrop; the parent unmounts it.
	`closable={false}` (e.g. while a create is pending) disables × and swallows Escape and a backdrop click without reporting onclose; a
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

	/** The press began on the backdrop: only then may the click that follows close the dialog. */
	let pressedOutside = false;

	function outside(e: MouseEvent): boolean {
		if (e.target !== el) return false; // a child of the dialog
		const r = el.getBoundingClientRect();
		return e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
	}

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

<!-- Escape is the keyboard equivalent of a backdrop click. -->
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
<dialog
	bind:this={el}
	onpointerdown={(e) => (pressedOutside = outside(e))}
	onclick={(e) => {
		const close = pressedOutside && outside(e);
		pressedOutside = false;
		if (close && closable) onclose();
	}}
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
