<!-- web/src/lib/components/Relations.svelte: an item's relations and its web links (kb/ADR-0031), on the initiative
     page, the epic page and in the story drawer. Two sections:
     "Relations": the blockers first (stories only: the drawer passes its own blocker list and controls as `blockers`,
     and this adds only the label), then, set apart under "Item links" where there are blockers to tell them from, the
     item links grouped by name and side, worded by the server's `label`.
     "Web links": each a title that opens the URL in a new tab, with its host beside it.
     Nothing here says whether a blocker currently holds the story back: that takes deliveries and owners this
     section does not have, and the readiness reason says it (kb/ADR-0027 R5). -->
<script lang="ts">
	import { tick, type Snippet } from 'svelte';
	import { page } from '$app/state';
	import { api, ApiError, unwrap } from '#lib/api/client.ts';
	import type { ItemLink, ItemType, ItemLinkType, SearchHit, WebLink } from '#lib/api/types.ts';
	import Glyph from '#lib/components/Glyph.svelte';
	import ItemSearch from '#lib/components/ItemSearch.svelte';
	import { focusIsFree } from '#lib/focus.ts';
	import { errorMessage } from '#lib/format.ts';
	import { PICKER_INPUT } from '#lib/picker.ts';
	import { followsNote, groupLinks, LINK_TYPES, linkedHref, linkTypeHint, linkTypeText, removal, webHost, type FollowsNote } from '#lib/relations.ts';

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
		/** Stories only: the host's blocker list and its controls, shown first under "blocked by". It is handed the id of
		 *  that label, which names its list (`aria-labelledby`). */
		blockers?: Snippet<[string]>;
		/** Called after a write here landed, so the host refetches its item. */
		onchanged?: () => void;
	} = $props();

	const id = $props.id();
	/** The host's lists, each a new value whenever the host passes its next read: a write compares the one it started
	 *  under with the one current when its answer arrives (`===`) to learn whether the host read meanwhile. */
	const hostLinks = $derived(links);
	const hostWeb = $derived(webLinks);
	/** What is listed: the host's lists. A write's answer is assigned over them and shows until the host passes its
	 *  next read, which replaces it. An answer that arrives after the host passed a read is not assigned: that read is
	 *  the newer one (a live reload, with what someone else added meanwhile), and it stays while the refetch that
	 *  `onchanged` asks for runs, and if that refetch fails. An add then puts the one row it added on that read, so a
	 *  confirmed addition is never hidden (by a read from before it, or by an equal read). */
	let shownLinks = $derived(hostLinks);
	let shownWeb = $derived(hostWeb);
	const groups = $derived(groupLinks(shownLinks));

	/** One write at a time, in both sections: a late answer can then never undo a newer write's. */
	let busy = $state(false);
	let linkSection = $state<HTMLElement>();

	let addingLink = $state(false);
	let linkType = $state<ItemLinkType>(LINK_TYPES[0]);
	/** What the form says above its controls for the name chosen; '' for a name that needs no hint. */
	const hint = $derived(linkTypeHint(linkType, level));
	let picked = $state<SearchHit | null>(null);
	/** A refused add, shown in the form; a failed removal, shown under the list. */
	let linkError = $state<string | null>(null);
	let listError = $state<string | null>(null);
	/** Set after a `follows` between two stories; cleared by the next write. */
	let note = $state<FollowsNote | null>(null);

	const linkForm = () => document.getElementById(`${id}-link-form`);
	const sameLink = (a: ItemLink, b: ItemLink) => a.type === b.type && a.direction === b.direction && a.item.key === b.item.key;

	/** Escape in a form cancels it. The key is prevented, so a host that closes on Escape (the story drawer) stays
	 *  open; an Escape the item search already used to close its list is left alone. */
	const cancelOnEscape = (close: () => void) => (e: KeyboardEvent) => {
		if (e.key !== 'Escape' || e.defaultPrevented || busy) return;
		e.preventDefault();
		close();
	};

	/** Whether focus is still where the press left it: on `pressed`, or on the page because that button gave it up
	 *  (disabled while the request was out, or gone with its row). Focus the person moved to anything else, another
	 *  control of this same section included, is theirs. `focusIsFree` is the wrong guard here: it would call the whole
	 *  section free. */
	const focusStayed = (pressed: HTMLElement): boolean => {
		const a = document.activeElement;
		return !a || a === document.body || a === pressed;
	};
	/** After a removal answered, once the DOM shows the result: focus goes to `target()` if it stayed where the press
	 *  left it, and is left alone otherwise. */
	async function refocus(pressed: HTMLElement, target: () => HTMLElement | null | undefined) {
		await tick();
		if (focusStayed(pressed)) target()?.focus();
	}
	/** A section's Remove buttons, in order (the host's blocker buttons carry no `remove` class). */
	const removes = (section: HTMLElement | undefined) => [...(section?.querySelectorAll<HTMLButtonElement>('button.remove') ?? [])];
	const linkRemoves = () => removes(linkSection);
	/** Where focus goes when a row's Remove was pressed at place `index` and that button is no longer there to take it:
	 *  the Remove now at that place, else the last one, else the section's add control (its button, or its open form's
	 *  first field). */
	const nextControl = (left: HTMLButtonElement[], index: number, add: () => HTMLElement | null | undefined) => left[Math.min(index, left.length - 1)] ?? add();
	const afterLink = (index: number) => nextControl(linkRemoves(), index, () => document.getElementById(`${id}-link-add`) ?? linkForm()?.querySelector<HTMLElement>('select'));
	/** After a removal failed: the button that was pressed, enabled again; when a reload took its row while the request
	 *  was out, a detached button takes no focus, so `next()` says where it goes instead. */
	const pressedOr = (pressed: HTMLButtonElement, next: () => HTMLElement | null | undefined) => () => (pressed.isConnected && !pressed.disabled ? pressed : next());

	async function openLink() {
		addingLink = true;
		await tick();
		linkForm()?.querySelector<HTMLElement>('select')?.focus();
	}

	/** Closes the form and drops its draft; focus returns to the add button unless the user moved it elsewhere. */
	async function closeLink() {
		const free = focusIsFree(linkForm());
		addingLink = false;
		picked = null;
		linkType = LINK_TYPES[0];
		linkError = null;
		await tick();
		if (free) document.getElementById(`${id}-link-add`)?.focus();
	}

	/** `POST /items/{key}/links`: this item follows, duplicates or relates to the picked one. Always the forward form.
	 *  The answer is this item's links. */
	async function addLink(e: SubmitEvent) {
		e.preventDefault();
		const other = picked;
		if (!other || busy) return;
		busy = true;
		linkError = null;
		listError = null;
		note = null;
		const under = hostLinks;
		const sent = linkType;
		let answer: ItemLink[];
		try {
			answer = await unwrap(api.POST('/api/v1/items/{key}/links', { params: { path: { key: itemKey } }, body: { type: sent, to: other.key } }));
		} catch (err) {
			// The fields are enabled again before focus returns to one: a disabled field cannot take it.
			busy = false;
			linkError = err instanceof ApiError && err.code === 'not_found' ? `No item ${other.key}.` : errorMessage(err);
			await tick();
			if (focusIsFree(linkForm())) linkForm()?.querySelector<HTMLElement>(PICKER_INPUT)?.focus();
			return;
		}
		busy = false;
		if (hostLinks === under) shownLinks = answer;
		else {
			// The host passed a newer list meanwhile (by identity, so possibly an equal one): it stays, with the one row
			// this write added. No other row of the answer comes back: someone may have removed it since.
			const mine = (l: ItemLink) => l.type === sent && l.item.key === other.key && (sent === 'relates_to' || l.direction === 'out');
			const added = answer.find(mine);
			if (added && !hostLinks.some(mine)) shownLinks = [...hostLinks, added];
		}
		note = followsNote(sent, itemKey, level, other);
		await closeLink();
		onchanged?.();
	}

	/** `DELETE /items/{key}/links/{type}/{to}` in the forward form (`removal` swaps the keys for an `in` link). The
	 *  answer is the list of the item named first in the path, which for an `in` link is the other item, so the row is
	 *  dropped here and the answer is not used. The row is dropped from the list on screen when the answer arrives
	 *  (`shownLinks` read after the await), not from the list as it was when the button was pressed: the host may have
	 *  passed a newer read meanwhile. */
	async function removeLink(l: ItemLink, pressed: HTMLButtonElement) {
		if (busy) return;
		busy = true;
		listError = null;
		note = null;
		const index = linkRemoves().indexOf(pressed);
		try {
			await unwrap(api.DELETE('/api/v1/items/{key}/links/{type}/{to}', { params: { path: removal(itemKey, l) } }));
		} catch (err) {
			busy = false;
			listError = `Could not remove the item link to ${l.item.key}: ${errorMessage(err)}`;
			// The row stays, and so does focus: back on the button that was pressed, now enabled again.
			await refocus(pressed, pressedOr(pressed, () => afterLink(index)));
			return;
		}
		shownLinks = shownLinks.filter((x) => !sameLink(x, l));
		busy = false;
		// The Remove that took the row's place, else the last one, else the section's add control.
		await refocus(pressed, () => afterLink(index));
		onchanged?.();
	}

	let addingWeb = $state(false);
	let title = $state('');
	let url = $state('');
	let webError = $state<string | null>(null);
	let webListError = $state<string | null>(null);

	let webSection = $state<HTMLElement>();
	const webForm = () => document.getElementById(`${id}-web-form`);
	const webAdd = () => document.getElementById(`${id}-web-add`) ?? webForm()?.querySelector<HTMLElement>('input');

	async function openWeb() {
		addingWeb = true;
		await tick();
		webForm()?.querySelector<HTMLElement>('input')?.focus();
	}

	async function closeWeb() {
		const free = focusIsFree(webForm());
		addingWeb = false;
		title = '';
		url = '';
		webError = null;
		await tick();
		if (free) document.getElementById(`${id}-web-add`)?.focus();
	}

	/** `POST /items/{key}/web-links`. The server validates the title and the URL (scheme, length, no user name or
	 *  password, the same URL under another title) and its message is shown as it is; the answer is this item's web
	 *  links, the URL as stored. */
	async function addWeb(e: SubmitEvent) {
		e.preventDefault();
		const body = { title: title.trim(), url: url.trim() };
		if (busy || !body.title || !body.url) return;
		busy = true;
		webError = null;
		webListError = null;
		const under = hostWeb;
		let answer: WebLink[];
		try {
			answer = await unwrap(api.POST('/api/v1/items/{key}/web-links', { params: { path: { key: itemKey } }, body }));
		} catch (err) {
			busy = false;
			webError = errorMessage(err);
			await tick();
			if (focusIsFree(webForm())) webForm()?.querySelector<HTMLElement>('input')?.focus();
			return;
		}
		busy = false;
		if (hostWeb === under) shownWeb = answer;
		else {
			// As for an item link: the newer list stays, with the one web link this write added. That one is known by the
			// title sent and by an id the newer list does not have; the URL is not compared, the server stores it
			// normalised.
			const added = answer.find((w) => w.title === body.title && !hostWeb.some((x) => x.id === w.id));
			if (added) shownWeb = [...hostWeb, added];
		}
		await closeWeb();
		onchanged?.();
	}

	/** `DELETE /items/{key}/web-links/{id}`: the answer is this item's web links. No confirmation: a web link can be
	 *  added again, and its activity entry keeps the title and the URL. When the host passed a read while the request
	 *  was out, the answer is not shown: the row is dropped from that read instead, which may be from before the
	 *  removal. */
	async function removeWeb(w: WebLink, pressed: HTMLButtonElement) {
		if (busy) return;
		busy = true;
		webListError = null;
		const under = hostWeb;
		const index = removes(webSection).indexOf(pressed);
		let answer: WebLink[];
		try {
			answer = await unwrap(api.DELETE('/api/v1/items/{key}/web-links/{id}', { params: { path: { key: itemKey, id: w.id } } }));
		} catch (err) {
			busy = false;
			webListError = `Could not remove the web link ${w.title}: ${errorMessage(err)}`;
			// The row stays, and so does focus: back on the button that was pressed.
			await refocus(pressed, pressedOr(pressed, () => nextControl(removes(webSection), index, webAdd)));
			return;
		}
		shownWeb = hostWeb === under ? answer : shownWeb.filter((x) => x.id !== w.id);
		busy = false;
		// To the section's add control (its button, or the open form's first field), never to another row's Remove.
		await refocus(pressed, webAdd);
		onchanged?.();
	}
</script>

<section class="relations" aria-labelledby="{id}-relations" bind:this={linkSection}>
	<svelte:element this={heading} id="{id}-relations" class="relations-heading">Relations</svelte:element>
	{#if blockers}
		<div class="group blocked">
			<p class="label" id="{id}-blocked">blocked by</p>
			<div class="items">{@render blockers(`${id}-blocked`)}</div>
		</div>
		<!-- Only beside blockers: the rows under it look like the blocker rows, and the standing line tells the two apart. -->
		<p class="sublabel">Item links</p>
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
						<button type="button" class="remove" aria-label="Remove item link: {l.label} {l.item.key}" disabled={busy} onclick={(e) => removeLink(l, e.currentTarget)}
							>Remove</button
						>
					</li>
				{/each}
			</ul>
		</div>
	{/each}
	{#if shownLinks.length === 0}<p class="muted empty">No item links yet. An item link says that this {level} follows, duplicates or relates to another item.</p>{/if}
	<!-- Always shown, and plain text (no live region): it is not news, it is what the section is. -->
	<p class="muted standing">{level === 'story' ? 'Item links are informational and never hold a story back. Of the relations listed here, only a blocker can.' : 'Item links are informational. They never hold work back.'}</p>
	{#if listError}<p class="error" role="alert">{listError}</p>{/if}
	<!-- Always present, so a screen reader hears the line arrive; a live region inserted with its text is skipped. Only
	     ever set in the story drawer (`followsNote`), where "+ Add blocker" is in the "blocked by" group above. -->
	<p class="informational" class:idle={!note} role="status">
		{#if note}Informational: this does not hold {note.key} back. To hold it back, add {note.by} with "+ Add blocker" above.{/if}
	</p>
	{#if addingLink}
		<!-- A named form is a landmark, which the linter calls non-interactive; the listener only catches Escape from the
		     fields inside it. -->
		<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
		<form id="{id}-link-form" class="add-form" aria-label="Add item link" onsubmit={addLink} onkeydown={cancelOnEscape(closeLink)}>
			<!-- First, on a row of its own above the controls: under them the open search list would cover it. -->
			{#if hint}<p class="muted hint" id="{id}-link-hint">{hint}</p>{/if}
			<!-- Reads as the sentence the link states: "This story follows <other item>". -->
			<span class="subject">This {level}</span>
			<select aria-label="Item link name" aria-describedby={hint ? `${id}-link-hint` : undefined} bind:value={linkType} disabled={busy}>
				{#each LINK_TYPES as t (t)}<option value={t}>{linkTypeText(t)}</option>{/each}
			</select>
			<ItemSearch label="Other item" placeholder="Search for an item" bind:picked exclude={itemKey} disabled={busy} />
			<!-- One flex child, so the two buttons wrap together and never part onto two rows. -->
			<span class="actions">
				<button type="button" disabled={busy} onclick={closeLink}>Cancel</button>
				<button type="submit" disabled={busy || !picked}>Add</button>
			</span>
			{#if linkError}<p class="error" role="alert">{linkError}</p>{/if}
		</form>
	{:else}
		<button type="button" class="ghost" id="{id}-link-add" onclick={openLink}>+ Add item link</button>
	{/if}
</section>

<section class="relations" aria-labelledby="{id}-web" bind:this={webSection}>
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
					<button type="button" class="remove" aria-label="Remove web link {w.title}" disabled={busy} onclick={(e) => removeWeb(w, e.currentTarget)}>Remove</button>
				</li>
			{/each}
		</ul>
	{/if}
	{#if webListError}<p class="error" role="alert">{webListError}</p>{/if}
	{#if addingWeb}
		<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
		<form id="{id}-web-form" class="add-form" aria-label="Add web link" onsubmit={addWeb} onkeydown={cancelOnEscape(closeWeb)}>
			<label class="field"
				>Title
				<input bind:value={title} disabled={busy} aria-invalid={webError ? 'true' : undefined} aria-describedby={webError ? `${id}-web-error` : undefined} />
			</label>
			<!-- The hint sits under the field, outside the label, so the field's name stays "URL". -->
			<div class="url">
				<label class="field"
					>URL
					<input
						bind:value={url}
						disabled={busy}
						inputmode="url"
						autocomplete="off"
						spellcheck="false"
						placeholder="https://"
						aria-invalid={webError ? 'true' : undefined}
						aria-describedby={webError ? `${id}-url-hint ${id}-web-error` : `${id}-url-hint`}
					/>
				</label>
				<p class="muted hint" id="{id}-url-hint">The full address, starting with https://</p>
			</div>
			<span class="actions">
				<button type="button" disabled={busy} onclick={closeWeb}>Cancel</button>
				<button type="submit" disabled={busy || !title.trim() || !url.trim()}>Add</button>
			</span>
			{#if webError}<p class="error" role="alert" id="{id}-web-error">{webError}</p>{/if}
		</form>
	{:else}
		<button type="button" class="ghost" id="{id}-web-add" onclick={openWeb}>+ Add web link</button>
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
	/* The one relation that can hold the story back reads in the text colour; the item link labels stay muted. */
	.blocked .label {
		color: var(--foreground);
	}
	/* Sets the item links apart from the blockers above: a hairline, and more room than between two groups. */
	.sublabel {
		margin-top: 8px;
		padding-top: 12px;
		border-top: 1px solid var(--line);
		font-size: 14px;
		font-weight: 600;
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
	/* At the row's end, also when a narrow row wraps it under the title. */
	.remove {
		margin-left: auto;
		height: 24px;
		padding: 0 8px;
		font-size: 13px;
	}
	/* The add buttons sit at the start of their row, not stretched across the grid. */
	/* Their text starts at the section's left edge, as the drawer's other add controls do; the small padding is room
	   for the hover background. */
	.ghost {
		justify-self: start;
		margin-left: -4px;
		padding: 0 4px;
	}
	/* A note under the rows, in their size. */
	.standing {
		font-size: 14px;
	}
	.actions {
		display: flex;
		gap: 8px;
	}
	/* Every control may shrink and wrap: at phone width the form stacks instead of scrolling the page sideways. */
	.add-form {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		min-width: 0;
	}
	.add-form select {
		max-width: 100%;
	}
	/* Said in words and announced (role="alert"); the colour only adds to it. */
	.error {
		flex-basis: 100%;
		color: var(--danger);
		font-size: 14px;
	}
	.informational {
		color: var(--muted);
		font-size: 14px;
		overflow-wrap: anywhere;
	}
	/* Empty, it takes no row of the grid, yet stays in the page for screen readers. */
	.informational.idle {
		position: absolute;
	}
	/* A hint takes a row of its own: above the item link controls, under the URL field. */
	.hint {
		flex-basis: 100%;
		font-size: 13px;
	}
	/* The two flex children: the title's label, and the URL's label with its hint under it. */
	.add-form > .field,
	.add-form > .url {
		flex: 1 1 12rem;
		min-width: 0;
	}
	.url {
		display: grid;
		gap: 2px;
	}
	.add-form .field input {
		width: 100%;
	}
</style>
