<script lang="ts">
	import type { Story } from '#lib/api/types.ts';

	type Criterion = Story['criteria'][number];

	// Edits a story's acceptance criteria locally; "save" hands the whole list, as shown, to `onsave`. The draft resets
	// whenever the saved criteria change (after a reload), so it always starts from what the server returned.
	let {
		storyKey,
		criteria,
		busy = false,
		locked = false,
		onsave,
		oncancel
	}: {
		storyKey: string;
		criteria: Criterion[];
		busy?: boolean;
		/** Its own save is pending: every field and Cancel are disabled, so nothing typed now is lost when it closes. */
		locked?: boolean;
		/** Called with every criterion, each with the `done` the editor shows; blank lines are left out. */
		onsave: (criteria: Criterion[]) => void;
		/** Given, a Cancel button closes the editor without saving. */
		oncancel?: () => void;
	} = $props();

	// Compared as text, so a reload that returns the same criteria (another story was saved) keeps this draft.
	const saved = $derived(JSON.stringify(criteria.map((c) => ({ text: c.text, done: c.done }))));
	let draft = $derived(JSON.parse(saved) as Criterion[]);
	let adding = $state('');

	const toSave = $derived(draft.filter((c) => c.text.trim()).map((c) => ({ text: c.text.trim(), done: c.done })));
	const dirty = $derived(JSON.stringify(toSave) !== saved);

	function set(i: number, patch: Partial<Criterion>) {
		draft = draft.map((c, j) => (j === i ? { ...c, ...patch } : c));
	}

	/** Moves the add input's text into the list. */
	function take() {
		const text = adding.trim();
		if (!text) return;
		draft = [...draft, { text, done: false }];
		adding = '';
	}

	function add(e: SubmitEvent) {
		e.preventDefault();
		take();
	}

	/** Save keeps a criterion typed but not yet added: it joins the list first, so it is saved (or, if the save fails,
	 *  stays in the list) rather than left behind in an editor that closes. */
	function save() {
		take();
		onsave(toSave);
	}
</script>

<div class="criteria">
	{#if draft.length === 0}
		<p class="empty">No acceptance criteria.</p>
	{/if}
	<ol>
		{#each draft as c, i (i)}
			<li>
				<input
					type="checkbox"
					checked={c.done}
					disabled={locked}
					aria-label="criterion {i + 1} done"
					onchange={(e) => set(i, { done: e.currentTarget.checked })}
				/>
				<input
					class="text"
					value={c.text}
					disabled={locked}
					aria-label="criterion {i + 1}"
					oninput={(e) => set(i, { text: e.currentTarget.value })}
				/>
				<button type="button" aria-label="remove criterion {i + 1}" disabled={locked} onclick={() => (draft = draft.filter((_, j) => j !== i))}
					>×</button
				>
			</li>
		{/each}
	</ol>
	<form class="add" onsubmit={add}>
		<input bind:value={adding} disabled={locked} placeholder="add criterion" aria-label="add criterion to {storyKey}" />
		<button type="submit" disabled={locked || !adding.trim()}>Add</button>
		{#if oncancel}<button type="button" disabled={locked} onclick={oncancel}>Cancel</button>{/if}
		<button type="button" disabled={busy || locked || (!dirty && !adding.trim())} onclick={save}>Save</button>
		{#if dirty}<span class="muted">unsaved</span>{/if}
	</form>
</div>

<style>
	.criteria {
		display: grid;
		gap: 0.3rem;
		font-size: 14px;
	}
	ol {
		display: grid;
		gap: 0.25rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	li,
	.add {
		display: flex;
		gap: 0.4rem;
		align-items: center;
	}
	.text,
	.add input {
		flex: 1;
		min-width: 0;
		font: inherit;
	}
	.empty,
	.muted {
		margin: 0;
		color: var(--muted);
	}
</style>
