<!-- web/src/lib/ui/Picker.svelte: one searchable picker for epics, initiatives and labels (GRA-641), an editable combobox
     with list autocomplete (WAI-ARIA APG). It narrows the options it is given and never fetches. Closed, Enter and
     Escape belong to the surrounding form or drawer; open, they belong to the picker. `value` may be bound (a form field)
     or passed one way with `onchange` (a URL filter: the pick shows until the URL answers, as a <select> does). -->
<script lang="ts">
	import { tick } from 'svelte';
	import { narrow, withCurrent, type PickerOption } from '#lib/picker.ts';

	let {
		label,
		options,
		value = $bindable(''),
		empty,
		disabled = false,
		placeholder,
		onchange
	}: {
		label: string;
		options: PickerOption[];
		value?: string;
		empty?: string;
		disabled?: boolean;
		placeholder?: string;
		onchange?: (value: string) => void;
	} = $props();

	const id = $props.id();
	let input: HTMLInputElement;
	let open = $state(false);
	let query = $state('');
	let active = $state(-1);

	const all = $derived(withCurrent(options, value));
	const shown = $derived<PickerOption[]>([
		...(empty !== undefined && query.trim() === '' ? [{ value: '', label: empty }] : []),
		...narrow(all, query)
	]);
	// `withCurrent` holds every non-empty value, so only '' falls through to the empty text.
	const text = $derived(open ? query : (all.find((o) => o.value === value)?.label ?? empty ?? ''));

	// The active option stays in sight in a scrolled list (jsdom has no scrollIntoView, hence the optional call).
	$effect(() => {
		if (open && active >= 0) document.getElementById(`${id}-opt-${active}`)?.scrollIntoView?.({ block: 'nearest' });
	});

	function show() {
		if (open) return;
		open = true;
		query = '';
		active = Math.max(0, shown.findIndex((o) => o.value === value));
	}
	async function hide() {
		open = false;
		query = '';
		active = -1;
		// Closed and still focused (Escape, a pick), the label is selected again, so the next key replaces it.
		await tick();
		if (document.activeElement === input) input.select();
	}
	function pick(o: PickerOption) {
		const changed = o.value !== value;
		value = o.value;
		void hide();
		if (!changed) return;
		// A <select> fires `change`; so does this, so a host listening for edits (lib/ui/Dialog.svelte) sees the pick.
		input.dispatchEvent(new Event('change', { bubbles: true }));
		onchange?.(o.value);
	}
	function keydown(e: KeyboardEvent) {
		const last = shown.length - 1;
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			if (!open) return show();
			const step = e.key === 'ArrowDown' ? 1 : -1;
			active = Math.min(last, Math.max(0, (active < 0 ? -1 : Math.min(active, last)) + step));
		} else if (e.key === 'Enter' && open) {
			e.preventDefault();
			const o = shown[Math.min(active, last)];
			if (o) pick(o);
			else void hide();
		} else if (e.key === 'Escape' && open) {
			e.preventDefault();
			e.stopPropagation();
			void hide();
		}
	}
</script>

<span class="picker">
	<input
		bind:this={input}
		type="text"
		role="combobox"
		aria-label={label}
		aria-expanded={open}
		aria-controls="{id}-list"
		aria-autocomplete="list"
		aria-activedescendant={open && active >= 0 && active < shown.length ? `${id}-opt-${active}` : undefined}
		autocomplete="off"
		spellcheck="false"
		{disabled}
		{placeholder}
		value={text}
		onfocus={(e) => e.currentTarget.select()}
		oninput={(e) => {
			if (!open) show();
			query = e.currentTarget.value;
			active = 0;
		}}
		onclick={show}
		onkeydown={keydown}
		onblur={hide}
	/>
	{#if open && shown.length}
		<!-- One mousedown handler keeps focus in the input for a press anywhere on the list, its padding and scrollbar
		     included; otherwise the input blurs and the list closes before the click lands. -->
		<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
		<ul class="list" id="{id}-list" role="listbox" aria-label={label} onmousedown={(e) => e.preventDefault()}>
			{#each shown as o, i (o.value)}
				<!-- svelte-ignore a11y_click_events_have_key_events (the input owns the keyboard: APG combobox) -->
				<li
					id="{id}-opt-{i}"
					role="option"
					aria-selected={i === active}
					class:current={o.value === value}
					onclick={(e) => {
						e.preventDefault();
						pick(o);
					}}
				>
					{o.label}
				</li>
			{/each}
		</ul>
	{:else if open}
		<p class="none">No match</p>
	{/if}
	<!-- Always present, so a screen reader hears the text change; a live region inserted with its text is skipped. -->
	<span class="visually-hidden" role="status">{open && !shown.length ? 'No match' : ''}</span>
</span>

<style>
	.picker {
		position: relative;
		display: inline-block;
		min-width: 12rem;
		max-width: 100%;
		vertical-align: top;
	}
	input {
		width: 100%;
		box-sizing: border-box;
		text-overflow: ellipsis;
	}
	/* Floating under the input, so opening it moves nothing (a press beside it lands where it was aimed). z-index 15:
	   over the drawer (10), under the sticky top bar (20) and ActionsMenu (30), so it slides under the bar on scroll.
	   A <dialog> lets it out while it is open (app.css). `width: 0; min-width: 100%` makes it exactly as wide as the
	   picker without letting a long option widen it. */
	.list,
	.none {
		position: absolute;
		top: 100%;
		left: 0;
		z-index: 15;
		box-sizing: border-box;
		width: 0;
		min-width: 100%;
		margin: 2px 0 0;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.list {
		max-height: 16rem;
		padding: 4px 0;
		overflow-y: auto;
		list-style: none;
	}
	li {
		padding: 6px 10px;
		overflow-wrap: anywhere;
		cursor: pointer;
	}
	li[aria-selected='true'] {
		background: var(--selection);
	}
	li.current {
		font-weight: 600;
	}
	.none {
		padding: 6px 10px;
		color: var(--muted);
	}
</style>
