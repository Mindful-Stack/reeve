<!-- web/src/lib/components/Relations.svelte: an item's relations and its web links (kb/ADR-0031), on the initiative
     page, the epic page and in the story drawer. Two sections:
     "Relations": the blockers first (stories only: the drawer passes its own blocker list and controls as `blockers`,
     and this adds only the label), then the item links grouped by name and side, worded by the server's `label`.
     "Web links": each a title that opens the URL in a new tab, with its host beside it.
     Nothing here says whether a blocker currently holds the story back: that takes deliveries and owners this
     section does not have, and the readiness reason says it (kb/ADR-0027 R5). -->
<script lang="ts">
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import type { ItemLink, ItemType, WebLink } from '#lib/api/types.ts';
	import Glyph from '#lib/components/Glyph.svelte';
	import { groupLinks, linkedHref, webHost } from '#lib/relations.ts';

	let {
		itemKey,
		level,
		links,
		webLinks,
		heading = 'h2',
		blockers,
		onchanged
	}: {
		/** The item's own key, never an external reference: writes name it in their path. */
		itemKey: string;
		level: ItemType;
		/** The item's `links` and `web_links` as its last read carried them. */
		links: readonly ItemLink[];
		webLinks: readonly WebLink[];
		/** The level of the two section headings: `h2` on a page, `h3` in the drawer. */
		heading?: 'h2' | 'h3';
		/** Stories only: the host's blocker list and its controls, shown first under "blocked by". */
		blockers?: Snippet;
		/** Called after a write here landed, so the host refetches its item. */
		onchanged?: () => void;
	} = $props();

	const id = $props.id();
	/** What is listed: the host's lists. A write's answer is assigned over them and shows until the host passes its
	 *  next read, which replaces it. */
	let shownLinks = $derived(links);
	let shownWeb = $derived(webLinks);
	const groups = $derived(groupLinks(shownLinks));
</script>

<section class="relations" aria-labelledby="{id}-relations">
	<svelte:element this={heading} id="{id}-relations" class="relations-heading">Relations</svelte:element>
	{#if blockers}
		<div class="group">
			<p class="label">blocked by</p>
			<div class="items">{@render blockers()}</div>
		</div>
	{/if}
	{#each groups as g (g.id)}
		<div class="group">
			<p class="label" id="{id}-{g.id}">{g.label}</p>
			<ul class="items" aria-labelledby="{id}-{g.id}">
				{#each g.links as l (l.item.key)}
					<li>
						<a href={linkedHref(page.url, l.item)} data-sveltekit-reset={l.item.level === 'story' ? 'false' : undefined}
							><span class="key"><Glyph level={l.item.level} />{l.item.key}</span> {l.item.title}</a
						>
					</li>
				{/each}
			</ul>
		</div>
	{/each}
	{#if shownLinks.length === 0}<p class="muted empty">No item links yet. An item link says that this {level} follows, duplicates or relates to another item.</p>{/if}
	<!-- Always shown, and plain text (no live region): it is not news, it is what the section is. -->
	<p class="muted standing">{level === 'story' ? 'Item links are informational. Only a blocker holds a story back.' : 'Item links are informational. They never hold work back.'}</p>
</section>

<section class="relations" aria-labelledby="{id}-web">
	<svelte:element this={heading} id="{id}-web" class="relations-heading">Web links</svelte:element>
	{#if shownWeb.length === 0}
		<p class="muted empty">No web links yet. A web link points at a page outside the planner.</p>
	{:else}
		<ul aria-labelledby="{id}-web">
			{#each shownWeb as w (w.id)}
				{@const host = webHost(w.url)}
				<li>
					{#if host !== null}
						<!-- The hidden text is an expression: Svelte drops the leading space of a text node that starts an element. -->
						<a href={w.url} target="_blank" rel="noopener noreferrer">{w.title}<span class="visually-hidden">{' (opens in a new tab)'}</span></a>
						<span class="muted host">{host}</span>
					{:else}
						<span>{w.title}</span>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</section>

<style>
	/* Colours come from app.css tokens only (light and dark), through `.muted` and `var(--…)`. */
	section {
		display: grid;
		gap: 8px;
		min-width: 0;
	}
	/* A label beside its items; at a narrow width the items wrap under the label instead of pushing the page wide. */
	.group {
		display: flex;
		flex-wrap: wrap;
		gap: 2px 12px;
		align-items: baseline;
		min-width: 0;
	}
	.label {
		flex: 0 0 8.5rem;
		margin: 0;
		color: var(--muted);
		font-size: 14px;
	}
	.items {
		flex: 1 1 14rem;
		min-width: 0;
	}
	ul {
		display: grid;
		gap: 4px;
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: 14px;
	}
	li {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 10px;
		align-items: baseline;
		min-width: 0;
	}
	/* A long title or an unbroken URL-like title wraps inside the row. */
	li a,
	li > span {
		min-width: 0;
		overflow-wrap: anywhere;
	}
	li a {
		flex: 1 1 12rem;
	}
	.host {
		font-size: 13px;
	}
	p {
		margin: 0;
	}
</style>
