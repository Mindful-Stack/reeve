<!-- web/src/lib/components/BacklogFilters.svelte: the backlog's filter bar. State, View, the text chip and the Filters (n)
     disclosure, then the page's own buttons (`children`), then the disclosure's panel. Every control edits the URL in
     place (#lib/backlog.ts reads it); the server filters (kb/ADR-0021 R1). -->
<script lang="ts">
	import type { Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import type { Epic, Initiative } from '#lib/api/types.ts';
	import { backlogFilters, backlogState, backlogText, backlogView, withoutFilters, type BacklogState, type BacklogView } from '#lib/backlog.ts';
	import { isKind, KINDS } from '#lib/board.ts';
	import { session } from '#lib/me.svelte.ts';
	import Toggle from '#lib/ui/Toggle.svelte';

	let {
		epics,
		initiatives,
		labels,
		children
	}: {
		epics: Epic[];
		initiatives: Initiative[];
		/** The Label select's options (the page's: the todo list's, the rows' and the selected one). */
		labels: string[];
		children?: Snippet;
	} = $props();

	const id = $props.id();
	const p = $derived(page.url.searchParams);
	const text = $derived(backlogText(page.url));
	const listState = $derived(backlogState(page.url));
	const view = $derived(backlogView(page.url));
	const active = $derived(Object.keys(backlogFilters(page.url, session.me?.email)).length);
	/** Open on load when a filter is set; afterwards the user's. */
	let open = $state(Object.keys(backlogFilters(page.url, session.me?.email)).length > 0);

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
	function dropText() {
		return edit((s) => {
			s.delete('q');
			if (s.get('view') === 'relevance') s.delete('view');
		});
	}
	const clearFilters = () => goto(withoutFilters(page.url), { replace: true, reset: false });

	const epic = $derived(p.get('epic') ?? '');
	const initiative = $derived(p.get('initiative') ?? '');
</script>

<Toggle
	label="State"
	options={[
		{ value: 'todo', label: 'Todo' },
		{ value: 'open', label: 'Open' },
		{ value: 'all', label: 'All' }
	]}
	value={listState}
	onchange={(v: BacklogState) => set('state', v === 'todo' ? null : v)}
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
	<button type="button" class="chip" aria-label="Remove the text filter “{text}”" onclick={dropText}>“{text}” <span aria-hidden="true">×</span></button>
{/if}
<button type="button" aria-expanded={open} aria-controls="{id}-filters" onclick={() => (open = !open)}>Filters{active ? ` (${active})` : ''}</button>
<span class="spacer"></span>
{@render children?.()}
{#if open}
	<div class="panel" id="{id}-filters" role="group" aria-label="Filters">
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
			<select value={epic} onchange={(e) => set('epic', e.currentTarget.value || null)}>
				<option value="">any</option>
				{#each epics as e (e.key)}<option value={e.key}>{e.key} {e.title}</option>{/each}
				{#if epic && !epics.some((e) => e.key === epic)}<option value={epic}>{epic}</option>{/if}
			</select>
		</label>
		<label class="field inline">
			Initiative
			<select value={initiative} onchange={(e) => set('initiative', e.currentTarget.value || null)}>
				<option value="">any</option>
				{#each initiatives as i (i.key)}<option value={i.key}>{i.key} {i.title}</option>{/each}
				{#if initiative && !initiatives.some((i) => i.key === initiative)}<option value={initiative}>{initiative}</option>{/if}
			</select>
		</label>
		<label class="field inline">
			Label
			<select value={p.get('label') ?? ''} onchange={(e) => set('label', e.currentTarget.value || null)}>
				<option value="">any</option>
				{#each labels as l (l)}<option value={l}>{l}</option>{/each}
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
		{#if active}<button type="button" onclick={clearFilters}>Clear filters</button>{/if}
	</div>
{/if}

<style>
	.spacer {
		flex: 1;
	}
	.chip {
		max-width: 16rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.panel {
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
	.inline {
		display: flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
	}
	select {
		max-width: 16rem;
		min-width: 0;
	}
</style>
