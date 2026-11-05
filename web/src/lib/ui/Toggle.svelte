<!-- web/src/lib/ui/Toggle.svelte: a segmented choice (aria-pressed buttons) or tabs (roving tabindex, arrow/Home/End keys).
	Tabs: pass `panel` (an id); the selected tab gets aria-controls={panel}. Callers render
	<div role="tabpanel" id={panel} aria-labelledby={selectedTabId}> and read the selected tab's id with bind:selectedTabId.
	onchange may return a promise (e.g. goto): keys move focus to the new tab once the parent applies the change, which may be
	after the promise settles; a pending focus request disarms when its result has settled plus a tick (only if no newer
	key press replaced it), so a rejected change leaves nothing armed; a rejection is rethrown, not swallowed. -->
<script lang="ts" generics="T extends string">
	import { tick } from 'svelte';

	let {
		label,
		options,
		value,
		onchange,
		mode = 'segmented',
		panel,
		selectedTabId = $bindable()
	}: {
		label: string;
		options: { value: T; label: string }[];
		value: T;
		onchange: (v: T) => void | Promise<unknown>;
		mode?: 'segmented' | 'tabs';
		panel?: string;
		selectedTabId?: string;
	} = $props();

	const id = $props.id();
	const selected = $derived(Math.max(0, options.findIndex((o) => o.value === value)));
	const current = $derived(options[selected]?.value);
	const tabId = (v: string) => `${id}-tab-${v}`;
	$effect(() => {
		selectedTabId = current === undefined ? undefined : tabId(current);
	});

	let seq = 0;
	let pending: { token: number; target: T } | null = null;
	let tablist: HTMLDivElement | undefined;

	function choose(v: T) {
		if (v !== value) return onchange(v);
	}

	function keydown(e: KeyboardEvent) {
		if (mode !== 'tabs' || options.length === 0) return;
		let n: number;
		if (e.key === 'ArrowRight') n = (selected + 1) % options.length;
		else if (e.key === 'ArrowLeft') n = (selected - 1 + options.length) % options.length;
		else if (e.key === 'Home') n = 0;
		else if (e.key === 'End') n = options.length - 1;
		else return;
		e.preventDefault();
		tablist = e.currentTarget as HTMLDivElement;
		if (n === selected) return;
		const token = ++seq;
		pending = { token, target: options[n].value };
		const disarm = () => {
			if (pending?.token === token) pending = null;
		};
		let r: void | Promise<unknown>;
		try {
			r = choose(options[n].value);
		} catch (err) {
			disarm();
			throw err;
		}
		Promise.resolve(r).then(
			() => tick().then(disarm),
			(err) => {
				disarm();
				throw err;
			}
		);
	}

	$effect(() => {
		const at = selected; // reading value and options through selected
		if (!pending || current !== pending.target) return;
		pending = null;
		if (tablist?.contains(document.activeElement))
			tablist.querySelectorAll<HTMLButtonElement>('[role="tab"]')[at]?.focus();
	});
</script>

{#if mode === 'tabs'}
	<div class="tabs" role="tablist" aria-label={label} tabindex="-1" onkeydown={keydown}>
		{#each options as o (o.value)}
			<button
				type="button"
				role="tab"
				id={tabId(o.value)}
				aria-controls={o.value === current ? panel : undefined}
				aria-selected={o.value === current}
				tabindex={o.value === current ? 0 : -1}
				onclick={() => choose(o.value)}>{o.label}</button
			>
		{/each}
	</div>
{:else}
	<div class="segmented" role="group" aria-label={label}>
		{#each options as o (o.value)}
			<button type="button" aria-pressed={o.value === value} onclick={() => choose(o.value)}>{o.label}</button>
		{/each}
	</div>
{/if}

<style>
	.segmented {
		display: inline-flex;
		gap: 4px;
		padding: 3px;
		border-radius: 5px;
		background: var(--lighter-background);
	}
	.segmented button {
		height: 26px;
		padding: 0 10px;
		border: 0;
		border-radius: 3px;
		background: transparent;
		color: var(--muted);
	}
	.segmented button[aria-pressed='true'] {
		background: var(--raised);
		color: var(--foreground);
	}
	.tabs {
		display: flex;
		gap: 4px;
		padding: 0 10px;
		border-bottom: 1px solid var(--line);
	}
	.tabs button {
		height: auto;
		margin-bottom: -1px;
		padding: 8px 10px;
		border: 0;
		border-bottom: 2px solid transparent;
		border-radius: 0;
		background: none;
		color: var(--muted);
	}
	.tabs button:hover:not(:disabled) {
		background: none;
		color: var(--foreground);
	}
	.segmented button[aria-pressed='true']:hover:not(:disabled) {
		background: var(--raised);
	}
	.segmented button[aria-pressed='false']:hover:not(:disabled) {
		background: transparent;
		color: var(--foreground);
	}
	.tabs button[aria-selected='true'] {
		border-bottom-color: var(--accent);
		color: var(--foreground);
	}
</style>
