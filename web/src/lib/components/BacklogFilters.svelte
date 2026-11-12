<!-- web/src/lib/components/BacklogFilters.svelte: the backlog's filter bar. State, View, the text chip, the Filters (n)
     disclosure and its panel (right after its trigger in DOM order, so Tab reaches the filters next; CSS `order` shows the
     panel on its own row below the page's buttons), then the page's own buttons (`children`). Every control edits the
     URL in place (#lib/backlog.ts reads it); the server filters (kb/ADR-0021 R1). -->
<script module lang="ts">
	import { tick } from 'svelte';
	import Picker from '#lib/ui/Picker.svelte';
	import { keyed } from '#lib/picker.ts';
	import { focusIsFree } from '#lib/focus.ts';

	/** After a control that removed itself (the chip's ×, Clear filters here or in the page's empty state): focus the
	 *  Filters button rather than let it drop to the page, unless the user has put it somewhere already. */
	export async function focusFiltersToggle(): Promise<void> {
		await tick();
		if (focusIsFree(null)) document.querySelector<HTMLElement>('[data-filters-toggle]')?.focus();
	}

	/** After a control that removed itself by changing the state (the empty state's Show open stories): focus the State
	 *  control's selected button, unless the user has put focus somewhere already. */
	export function focusStateToggle(): void {
		if (focusIsFree(null)) document.querySelector<HTMLElement>('[role="group"][aria-label="State"] button[aria-pressed="true"]')?.focus();
	}
</script>

<script lang="ts">
	import type { Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import type { Epic, Initiative } from '#lib/api/types.ts';
	import { activeFilterCount, backlogState, backlogText, backlogView, stateParam, withoutFilters, withoutText, withPromised, type BacklogState, type BacklogView } from '#lib/backlog.ts';
	import { isKind, KINDS } from '#lib/board.ts';
	import { session } from '#lib/me.svelte.ts';
	import { isPromisedFilter, PROMISED_NONE, PROMISED_OPTIONS } from '#lib/promise.ts';
	import Toggle from '#lib/ui/Toggle.svelte';

	let {
		epics,
		initiatives,
		labels,
		children
	}: {
		epics: Epic[];
		initiatives: Initiative[];
		/** The Label picker's options (the page's: the todo list's, the rows' and the selected one). */
		labels: string[];
		children?: Snippet;
	} = $props();

	const id = $props.id();
	const p = $derived(page.url.searchParams);
	const text = $derived(backlogText(page.url));
	const listState = $derived(backlogState(page.url));
	const view = $derived(backlogView(page.url));
	const active = $derived(activeFilterCount(page.url, session.me?.email));
	/** Open on load when a filter is set; afterwards the user's. */
	let open = $state(activeFilterCount(page.url, session.me?.email) > 0);

	/** Edits the URL in place: no history entry, scroll and focus kept. */
	function edit(change: (s: URLSearchParams) => void): Promise<void> {
		const url = new URL(page.url.href);
		change(url.searchParams);
		return goto(url, { replace: true, reset: false });
	}
	const set = (name: string, value: string | null) => edit((s) => (value ? s.set(name, value) : s.delete(name)));
	const flag = (name: string) => (e: Event & { currentTarget: HTMLInputElement }) => set(name, e.currentTarget.checked ? '1' : null);

	function setView(v: BacklogView) {
		return edit((s) => {
			s.set('view', v);
			s.delete('group'); // the legacy spelling of view=rank
		});
	}
	/** Removes a control (the chip, Clear filters) from under the focus, then hands it to the Filters button. */
	async function away(to: URL) {
		await goto(to, { replace: true, reset: false });
		await focusFiltersToggle();
	}

	const epic = $derived(p.get('epic') ?? '');
	const initiative = $derived(p.get('initiative') ?? '');
	/** The Epic picker's options: the chosen initiative's epics (all of them when none is chosen, or when the URL names
	 *  one by a reference the list does not hold), plus the chosen epic. */
	const epicOptions = $derived(
		initiative && initiatives.some((i) => i.key === initiative) ? epics.filter((e) => e.initiative === initiative || e.key === epic) : epics
	);
</script>

<Toggle
	label="State"
	options={[
		{ value: 'todo', label: 'Todo' },
		{ value: 'open', label: 'Open' },
		{ value: 'all', label: 'All' }
	]}
	value={listState}
	onchange={(v: BacklogState) => set('state', stateParam(v, page.url))}
/>
<Toggle
	label="View"
	options={[
		{ value: 'epic', label: 'By epic' },
		{ value: 'rank', label: 'Ranked' },
		...(text ? [{ value: 'relevance' as const, label: 'Relevance' }] : [])
	]}
	value={view}
	onchange={setView}
/>
{#if text}
	<button type="button" class="chip" aria-label="Remove the text filter “{text}”" onclick={() => away(withoutText(page.url))}
		><span class="text">“{text}”</span> <span class="x" aria-hidden="true">×</span></button
	>
{/if}
<button type="button" data-filters-toggle aria-expanded={open} aria-controls="{id}-filters" onclick={() => (open = !open)}
	>Filters{active ? ` (${active})` : ''}</button
>
<div class="panel" id="{id}-filters" role="group" aria-label="Filters" hidden={!open}>
	<label class="check">
		<input type="checkbox" checked={p.get('mine') === '1'} onchange={flag('mine')} />
		Mine
	</label>
	<label class="field inline">
		Kind
		<select value={isKind(p.get('kind')) ? p.get('kind') : ''} onchange={(e) => set('kind', e.currentTarget.value || null)}>
			<option value="">any</option>
			{#each KINDS as k (k)}<option value={k}>{k}</option>{/each}
		</select>
	</label>
	<label class="field inline">
		Epic
		<Picker label="Epic" options={keyed(epicOptions)} value={epic} empty="any" onchange={(v) => set('epic', v || null)} />
	</label>
	<label class="field inline">
		Initiative
		<Picker label="Initiative" options={keyed(initiatives)} value={initiative} empty="any" onchange={(v) => set('initiative', v || null)} />
	</label>
	<label class="field inline">
		Label
		<Picker
			label="Label"
			options={labels.map((l) => ({ value: l, label: l }))}
			value={p.get('label') ?? ''}
			empty="any"
			onchange={(v) => set('label', v || null)}
		/>
	</label>
	<label class="field inline">
		Promise
		<select
			value={isPromisedFilter(p.get('promised')) ? p.get('promised') : ''}
			onchange={(e) => {
				const v = e.currentTarget.value;
				return goto(withPromised(page.url, isPromisedFilter(v) ? v : null), { replace: true, reset: false });
			}}
		>
			<option value="">{PROMISED_NONE}</option>
			{#each PROMISED_OPTIONS as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
		</select>
	</label>
	<label class="check">
		<input type="checkbox" checked={p.get('ready') === '1'} onchange={flag('ready')} />
		Ready only
	</label>
	<label class="check">
		<input type="checkbox" checked={p.get('untriaged') === '1'} onchange={flag('untriaged')} />
		Untriaged
	</label>
	<label class="check">
		<input type="checkbox" checked={p.get('urgent') === '1'} onchange={flag('urgent')} />
		Urgent
	</label>
	{#if active}<button type="button" onclick={() => away(withoutFilters(page.url))}>Clear filters</button>{/if}
</div>
<span class="spacer"></span>
{@render children?.()}

<style>
	.spacer {
		flex: 1;
	}
	/* A long text ellipsises inside the chip; the × always shows. */
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		max-width: 16rem;
		min-width: 0;
	}
	.chip .text {
		min-width: 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.chip .x {
		flex: none;
	}
	/* After the page's buttons on screen, after the Filters button in the DOM (and so in the tab order). */
	.panel[hidden] {
		display: none;
	}
	.panel {
		order: 1;
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
		flex-basis: 100%;
		min-width: 0;
	}
	.check {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		color: var(--muted);
	}
	/* A long option (an epic's key and title) shrinks the picker, not the page: at 320px the label fits the panel. */
	.inline {
		display: flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
		max-width: 100%;
	}
	select {
		flex: 0 1 auto;
		max-width: 16rem;
		min-width: 0;
	}
</style>
