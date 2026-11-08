<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { SearchHit } from '#lib/api/types.ts';
	import Glyph from '#lib/components/Glyph.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import { highlight, hitHref, hitLevel } from '#lib/search.ts';

	// The query lives in the URL (?q=), so a search can be linked and survives a reload.
	const q = $derived(page.url.searchParams.get('q')?.trim() ?? '');
	let draft = $state('');
	let hits = $state<SearchHit[] | null>(null);
	let error = $state<string | null>(null);
	let latest = 0;
	let attempt = $state(0);

	// The item the query names by key (`12`, `ABC-12`) heads the page, above the levels; each level keeps the
	// server's order (exact reference, then titles, then the rest).
	const groups = $derived.by(() => {
		const all = hits ?? [];
		const byKey = all.filter((h) => h.matched.includes('key'));
		const rest = all.filter((h) => !h.matched.includes('key'));
		return [
			{ label: 'Key', hits: byKey },
			...(
				[
					['initiative', 'Initiatives'],
					['epic', 'Epics'],
					['story', 'Stories']
				] as const
			).map(([item, label]) => ({ label, hits: rest.filter((h) => h.item === item) }))
		].filter((g) => g.hits.length > 0);
	});

	// The field follows the URL only when the query changes, so a Retry never wipes what is being typed.
	$effect(() => {
		draft = q;
	});

	$effect(() => {
		const text = q;
		void attempt;
		hits = null;
		error = null;
		// Advanced before the empty-query return too, so a search still pending when the query is cleared is dropped.
		const seq = ++latest;
		if (!text) return;
		unwrap(api.GET('/api/v1/search', { params: { query: { q: text } } }))
			.then((h) => {
				if (seq === latest) hits = h;
			})
			.catch((e: unknown) => {
				if (seq === latest) error = e instanceof Error ? e.message : String(e);
			});
	});

	function submit(e: SubmitEvent) {
		e.preventDefault();
		const text = draft.trim();
		if (text) goto(`/search?q=${encodeURIComponent(text)}`);
	}
</script>

<svelte:head><title>Search · Reeve</title></svelte:head>

<PageHeader title="Search" />
<form role="search" aria-label="Search items" onsubmit={submit}>
	<label for="search-q">Title, brief, description, key or external reference</label>
	<input id="search-q" type="search" bind:value={draft} placeholder="e.g. 12, NorthCloud or JPD-30" />
	<button type="submit">Search</button>
</form>

{#if error}
	<Notice tone="error" onretry={() => attempt++}>Could not search: {error}</Notice>
{:else if q && hits === null}
	<p class="muted" role="status">Searching…</p>
{:else if hits && hits.length === 0}
	<p class="muted">Nothing found for “{q}”</p>
{:else if hits}
	{#each groups as g (g.label)}
		<section>
			<h2>{g.label} <span class="count">{g.hits.length}</span></h2>
			<ul class="rows">
				{#each g.hits as h (h.key)}
					<li>
						<a href={hitHref(h)}>
							<span class="key"><Glyph level={hitLevel(h)} />{h.key}</span>
							{#each highlight(h.title, q) as p, i (i)}{#if p.match}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}
						</a>
						<span class="muted small">matched {h.matched.map((m) => (m === 'external_ref' ? 'external reference' : m)).join(', ')}</span>
					</li>
				{/each}
			</ul>
		</section>
	{/each}
{/if}

<style>
	form {
		display: flex;
		gap: 0.5rem;
		align-items: center;
		flex-wrap: wrap;
		margin-bottom: 1rem;
	}
	input {
		min-width: 20rem;
		max-width: 100%;
	}
	section {
		margin-bottom: 18px;
	}
	h2 {
		margin-bottom: 8px;
	}
	.rows > li {
		gap: 4px 12px;
		align-items: baseline;
		justify-content: space-between;
	}
</style>
