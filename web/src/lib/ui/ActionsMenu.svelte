<!-- web/src/lib/ui/ActionsMenu.svelte: a button opening a menu of actions. The menu is rendered only while open and
     positioned fixed from the trigger, so a scrolling board column cannot clip it. No Popover API (jsdom lacks it).
     A `trigger` snippet replaces the default icon button content and gets no icon styling. -->
<script lang="ts" module>
	export type MenuItem = { label: string; onselect: () => void; disabled?: boolean };
</script>

<script lang="ts">
	import { tick, type Snippet } from 'svelte';

	let {
		label,
		items,
		control,
		trigger,
		field,
		class: cls = ''
	}: {
		label: string;
		items: MenuItem[];
		control?: string;
		trigger?: Snippet;
		/** The trigger's `data-field`, for a list that remembers focus by field name (DecisionList). */
		field?: string;
		class?: string;
	} = $props();

	const id = $props.id();
	let open = $state(false);
	let button: HTMLButtonElement;
	let menu: HTMLDivElement | undefined = $state();
	let pos = $state<{ top: number; left: number; maxHeight: number | null }>({ top: 0, left: 0, maxHeight: null });

	const enabled = () => [...(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? [])];

	function place() {
		if (!button) return;
		const r = button.getBoundingClientRect();
		// The sticky top bar publishes its height. A page menu's trigger scrolled behind the bar is out of sight, and its
		// menu stays below the bar. A menu whose trigger is in the bar (the account menu) is exempt from both.
		const inBar = !!button.closest('header.top-bar');
		const barBottom = inBar ? 0 : parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-bar-height')) || 0;
		if (open && (r.bottom < barBottom || r.top > window.innerHeight || r.right < 0 || r.left > window.innerWidth)) {
			open = false;
			button.focus({ preventScroll: true });
			return;
		}
		const m = menu?.getBoundingClientRect();
		const w = m?.width || 200;
		// The natural height, not the capped one, so a menu that grows back is measured again.
		const h = menu && menu.scrollHeight ? menu.scrollHeight + (menu.offsetHeight - menu.clientHeight) : m?.height || 0;
		const minTop = barBottom + 8;
		const spaceBelow = window.innerHeight - 8 - (r.bottom + 4);
		const spaceAbove = r.top - 4 - minTop;
		const below = h <= spaceBelow || (h > spaceAbove && spaceBelow >= spaceAbove);
		const space = below ? spaceBelow : spaceAbove;
		const used = Math.min(h, Math.max(space, 0));
		pos = {
			top: below ? r.bottom + 4 : r.top - 4 - used,
			left: Math.max(8, Math.min(r.left, window.innerWidth - w - 8)),
			maxHeight: h > space ? Math.max(space, 0) : null
		};
	}

	async function show() {
		place();
		open = true;
		await tick();
		place();
		(enabled()[0] ?? menu)?.focus();
	}

	function hide(refocus: boolean) {
		open = false;
		if (refocus) button.focus();
	}

	function keydown(e: KeyboardEvent) {
		const list = enabled();
		const i = list.indexOf(document.activeElement as HTMLButtonElement);
		const go = (n: number) => list[(n + list.length) % list.length]?.focus();
		if (e.key === 'ArrowDown') go(i + 1);
		else if (e.key === 'ArrowUp') go(i < 0 ? list.length - 1 : i - 1);
		else if (e.key === 'Home') go(0);
		else if (e.key === 'End') go(list.length - 1);
		else if (e.key === 'Escape') hide(true);
		else if (e.key === 'Tab') return hide(false);
		else return;
		e.preventDefault();
	}

	function choose(item: MenuItem) {
		hide(true);
		item.onselect();
	}

	$effect(() => {
		if (!open) return;
		const outside = (e: MouseEvent) => {
			const t = e.target as Node;
			if (menu?.contains(t) || button.contains(t)) return;
			const el = t instanceof Element ? t : t.parentElement;
			const focusable = el?.closest(
				'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]'
			);
			if (focusable) return hide(false);
			e.preventDefault();
			open = false;
			button.focus({ preventScroll: true });
		};
		document.addEventListener('mousedown', outside);
		window.addEventListener('scroll', place, { capture: true, passive: true });
		window.addEventListener('resize', place);
		return () => {
			document.removeEventListener('mousedown', outside);
			window.removeEventListener('scroll', place, { capture: true });
			window.removeEventListener('resize', place);
		};
	});
</script>

<button
	bind:this={button}
	type="button"
	class="menu-trigger {cls}"
	class:icon-trigger={!trigger}
	aria-haspopup="menu"
	aria-expanded={open}
	aria-controls={open ? id : undefined}
	aria-label={label}
	data-control={control}
	data-field={field}
	onclick={() => (open ? hide(true) : show())}
>
	{#if trigger}{@render trigger()}{:else}<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.6" /><circle cx="12" cy="12" r="1.6" /><circle cx="19" cy="12" r="1.6" /></svg>{/if}
</button>
{#if open}
	<div bind:this={menu} {id} class="menu" role="menu" aria-label={label} tabindex="-1" style="top: {pos.top}px; left: {pos.left}px;{pos.maxHeight === null ? '' : ` max-height: ${pos.maxHeight}px;`}" onkeydown={keydown}>
		{#each items as item, i (i)}
			<button type="button" role="menuitem" tabindex="-1" disabled={item.disabled} onclick={() => choose(item)}>{item.label}</button>
		{/each}
	</div>
{/if}

<style>
	.icon-trigger {
		width: 28px;
		height: 28px;
		padding: 0;
		justify-content: center;
		border-color: transparent;
		background: transparent;
		color: var(--muted);
	}
	.menu {
		position: fixed;
		box-sizing: border-box;
		max-height: calc(100vh - 16px);
		overflow-y: auto;
		z-index: 30;
		display: grid;
		min-width: 200px;
		max-width: 240px;
		padding: 4px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
		box-shadow: var(--shadow);
	}
	.menu button {
		justify-content: flex-start;
		border: 0;
		background: transparent;
		text-align: left;
	}
	.menu button:hover:not(:disabled),
	.menu button:focus-visible {
		background: var(--lighter-background);
	}
</style>
