<!-- web/src/routes/backlog/+page.svelte: the ranked todo list and the home of every epic. Readiness is the
     server's (kb/ADR-0018 R1); a refused move raises the interactive override prompt (R2). -->
<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import { storyHref } from '#lib/drawer.ts';
	import type { Epic, Initiative, Reason, StoryWithReadiness } from '#lib/api/types.ts';
	import { backlogFilters, backlogState, backlogText, backlogView, groupBacklog, toStoryQuery, withoutFilters } from '#lib/backlog.ts';
	import { COLUMN_LABEL, fullLabel, orderAfterDrop, resolutionsFor, type Column, type Resolution } from '#lib/board.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { session } from '#lib/me.svelte.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { moveStep, postMove } from '#lib/moves.ts';
	import { rankStory } from '#lib/rank.ts';
	import BacklogFilters from '#lib/components/BacklogFilters.svelte';
	import BacklogList from '#lib/components/BacklogList.svelte';
	import NewEpicForm from '#lib/components/NewEpicForm.svelte';
	import OverridePrompt from '#lib/components/OverridePrompt.svelte';
	import StoryForm from '#lib/components/StoryForm.svelte';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';

	/** What the server last answered, all from one request: the complete todo list in rank order (ranking places against
	 *  it, kb/ADR-0021), the rows the URL's query asked for (the todo list itself for the plain backlog) and the query they
	 *  belong to (`key`, serialised). Nothing changes until a reload: no optimistic moves, no filtering in the client. */
	let loaded = $state<{ key: string; full: StoryWithReadiness[]; rows: StoryWithReadiness[] } | null>(null);
	let epics = $state<Epic[]>([]);
	let initiatives = $state<Initiative[]>([]);
	/** The last fetch failed and there are no rows for the current query to show (the first load, or a new query). */
	let loadError = $state<string | null>(null);
	/** A refetch of the shown query failed: the list stays; the notice offers a retry. */
	let stale = $state<string | null>(null);
	/** What the last write said (a refusal, a conflict, an error). */
	let notice = $state<string | null>(null);
	let busy = $state(false);
	/** The open create dialog. Each opening gets its own id, so a create that answers late closes only the dialog it was
	 *  sent from, never one opened since. */
	let creating = $state<{ kind: 'story' | 'epic'; id: number } | null>(null);
	/** The open create form has a create in flight: its dialog cannot be dismissed until the answer is in. */
	let formBusy = $state(false);
	let openings = 0;
	/** A moved row whose focus waits for an applied reload: gone from the shown rows, `apply` moves focus on; still shown
	 *  (a move under Open, an epic change the filter keeps), its actions trigger is refocused. A superseded reload does
	 *  neither. */
	let leaving: { key: string; apply: () => void } | null = null;
	/** Read by screen readers after a rank change or a filter change. */
	let announce = $state('');

	/** Empties the live region first, so a message equal to the last one is read again. */
	async function say(text: string) {
		announce = '';
		await tick();
		announce = text;
	}
	type Move = { item: StoryWithReadiness; to: Column; resolution?: Resolution };
	/** A move into done waiting for its resolution. */
	let resolving = $state<Move | null>(null);
	/** A move refused for readiness reasons, waiting for an override reason. */
	let override = $state<(Move & { reasons: Reason[] }) | null>(null);
	/** The row whose epic is being changed, and the epic chosen in the dialog ('' is no epic). */
	let changingEpic = $state<{ item: StoryWithReadiness; epic: string } | null>(null);

	// State, view, text and filters live in the query string so they survive a reload (#lib/backlog.ts reads them).
	const storyQuery = $derived(toStoryQuery(page.url, session.me?.email));
	/** The query as a string: equal queries are equal keys, so a change elsewhere in the URL (the drawer) fetches nothing. */
	const queryKey = $derived(JSON.stringify(storyQuery));
	const listState = $derived(backlogState(page.url));
	const view = $derived(backlogView(page.url));
	const text = $derived(backlogText(page.url));
	const filtered = $derived(text !== null || Object.keys(backlogFilters(page.url, session.me?.email)).length > 0);
	const full = $derived(loaded?.full ?? null);
	const rows = $derived(loaded?.rows ?? []);
	/** The shown rows answer an older query: they offer no action until the current one lands. */
	const pending = $derived(loaded !== null && loaded.key !== queryKey);
	/** Ranking is offered in Todo, not by relevance, and only for the rows of the current query. */
	const rankable = $derived(!pending && listState === 'todo' && view !== 'relevance');
	const backlog = $derived(groupBacklog(session.config?.horizon_buckets ?? [], initiatives, epics, rows, { filtered }));
	const epicTitle = $derived(new Map(epics.map((e) => [e.key, e.title])));
	const stories = (n: number) => `${n} ${n === 1 ? 'story' : 'stories'}`;
	/** The header's count: "3 of 240" in Todo with filters or text, "3" in plain Todo, "3 stories" in Open and All. */
	const countText = $derived.by(() => {
		if (!loaded) return undefined;
		const n = loaded.rows.length;
		if (listState !== 'todo') return stories(n);
		return filtered ? `${n} of ${loaded.full.length}` : `${n}`;
	});
	/** The Label filter's options: the labels of the todo list and of the rows, and the selected one. */
	const labels = $derived.by(() => {
		const all = new Set<string>();
		for (const s of [...(full ?? []), ...rows]) for (const l of s.story.labels) all.add(l);
		const selected = page.url.searchParams.get('label');
		if (selected) all.add(selected);
		return [...all].sort((a, b) => a.localeCompare(b));
	});
	const matches = (n: number) => `${n} ${n === 1 ? 'match' : 'matches'}`;
	const searchAll = $derived(text ? `/search?q=${encodeURIComponent(text)}` : null);

	function clearFilters() {
		goto(withoutFilters(page.url), { replace: true, reset: false });
	}

	let latest = 0;

	/** A write's own reload, a live refetch and a query change can overlap; only the latest request's answer (or failure)
	 *  lands. Reads the URL's query when called. Returns the todo list this request fetched, in rank order, even when a
	 *  newer request superseded it (null on failure), so a write can read its own result. */
	async function reload(): Promise<StoryWithReadiness[] | null> {
		const seq = ++latest;
		const sq = storyQuery;
		const key = queryKey;
		try {
			const [f, r, e, i] = await Promise.all([
				unwrap(api.GET('/api/v1/stories', { params: { query: { delivery: 'todo' } } })),
				sq ? unwrap(api.GET('/api/v1/stories', { params: { query: sq } })) : null,
				unwrap(api.GET('/api/v1/epics')),
				unwrap(api.GET('/api/v1/initiatives'))
			]);
			if (seq !== latest) return f;
			const before = loaded?.key;
			const shown = r ?? f;
			loaded = { key, full: f, rows: shown };
			if (leaving) {
				const l = leaving;
				leaving = null;
				const still = shown.some((s) => s.story.key === l.key);
				// Once the row is gone from the page, and unless the user has put focus somewhere else meanwhile.
				void tick().then(() => {
					if (still) void refocus(l.key);
					else if (focusIsFree(null)) l.apply();
				});
			}
			epics = e;
			initiatives = i;
			loadError = stale = null;
			// A filter change is announced with the new count (the first load is not).
			if (before !== undefined && before !== key) void say(listState !== 'todo' || !filtered ? stories(shown.length) : `${shown.length} of ${stories(f.length)}`);
			return f;
		} catch (err) {
			if (seq !== latest) return null;
			const m = errorMessage(err);
			if (loaded?.key === key) stale = `Could not reload the backlog: ${m}`;
			else {
				loaded = null;
				loadError = m;
			}
			return null;
		}
	}

	// The first load, and every change of the query (not of the rest of the URL).
	$effect(() => {
		void queryKey;
		untrack(reload);
	});
	// Any activity entry, and every (re)connect, refetches with the current query: the dependency graph is the whole tree
	// (#lib/events.svelte.ts, kb/ADR-0011 R10).
	onLive(reload);

	/** Shows what went wrong; a conflict also reloads. */
	async function failed(e: unknown) {
		if (e instanceof Refused) notice = e.reasons.map((r) => fullLabel(r)).join('; ');
		else if (e instanceof Conflict) {
			notice = 'Someone else changed this story; the backlog was reloaded.';
			await reload();
		} else if (e instanceof ApiError) notice = e.message;
		else notice = errorMessage(e);
	}

	const rowOf = (key: string) =>
		[...document.querySelectorAll<HTMLElement>('li[data-key]')].find((el) => el.dataset.key === key);

	/** After a write and its reload, focus the row's actions trigger (the menu closed), or nothing if the row is gone. */
	async function refocus(key: string) {
		await tick();
		rowOf(key)?.querySelector<HTMLElement>('[data-control="menu"]')?.focus();
	}

	/** Places `key` before `before` in the complete todo list (filters only hide rows); a no-op sends nothing. */
	async function rank(key: string, before: string | null) {
		const list0 = full;
		if (!list0 || busy || !rankable || !orderAfterDrop(list0, key, before)) return;
		notice = null;
		leaving = null;
		busy = true;
		try {
			await rankStory(list0, key, before);
			// This write's own answer: a live refetch may have overtaken it, landed or not.
			const list = (await reload()) ?? full ?? [];
			const at = list.findIndex((s) => s.story.key === key);
			if (at !== -1) void say(`${key} moved to ${at + 1} of ${list.length}`);
		} catch (e) {
			await failed(e);
			if (!(e instanceof Conflict)) await reload();
		} finally {
			busy = false;
		}
		await refocus(key);
	}

	function open(kind: 'story' | 'epic') {
		formBusy = false;
		creating = { kind, id: ++openings };
	}

	/** The create callback of the dialog opened as `id`: closes that dialog only if it is still the open one. */
	function created(id: number): () => Promise<void> {
		return async () => {
			if (creating?.id === id) creating = null;
			await reload();
		};
	}

	/** Where focus goes once `key`'s row leaves the shown rows (read from the page before the write): the next row's
	 *  actions trigger in the same list, else the previous row's, else the group's heading, else the page heading. `reload`
	 *  applies it once an applied answer no longer shows the row. */
	function focusAfterLeaving(key: string): () => void {
		const li = rowOf(key);
		const next = (li?.nextElementSibling as HTMLElement | null)?.dataset.key;
		const prev = (li?.previousElementSibling as HTMLElement | null)?.dataset.key;
		const heading = li?.closest('.group')?.querySelector<HTMLElement>('h3') ?? li?.closest('section')?.querySelector<HTMLElement>('h2');
		return () => {
			if (rowOf(key)) return;
			const row = (next && rowOf(next)) || (prev && rowOf(prev));
			if (row) return row.querySelector<HTMLElement>('[data-control="menu"]')?.focus();
			if (heading?.isConnected) return heading.focus();
			const h1 = document.querySelector<HTMLElement>('.page h1');
			h1?.setAttribute('tabindex', '-1');
			h1?.focus();
		};
	}

	/** Every row move goes through here: a move into done first asks for the resolution (as on the board). */
	function startMove(item: StoryWithReadiness, to: Column) {
		if (busy || pending) return;
		const step = moveStep(item.story, to);
		if (step === 'resolve') resolving = { item, to };
		else if (step === 'move') move({ item, to });
	}

	async function move(m: Move, overrideReason?: string) {
		if (busy) return;
		const { item, to } = m;
		busy = true;
		notice = null;
		leaving = null;
		let moved = false;
		try {
			const out = await postMove(item.story, to, { resolution: m.resolution, overrideReason });
			if (out.kind === 'override') override = { ...m, reasons: out.reasons };
			else {
				override = null;
				if (out.kind === 'notice') notice = out.notice;
				else {
					void say(`${item.story.key} moved to ${COLUMN_LABEL[to]}`);
					moved = true;
					leaving = { key: item.story.key, apply: focusAfterLeaving(item.story.key) };
					await reload();
				}
			}
		} catch (e) {
			override = null;
			await failed(e);
		} finally {
			busy = false;
		}
		// The override prompt takes focus while it is open; a moved row's focus waits for the reload that lands.
		if (!moved && !override) await refocus(item.story.key);
	}

	/** Change epic: `PATCH /api/v1/stories/{key}` with only the epic, as the drawer's Epic field sends it. */
	async function saveEpic(e: SubmitEvent) {
		e.preventDefault();
		const c = changingEpic;
		if (!c || busy) return;
		const s = c.item.story;
		if (c.epic === (s.epic ?? '')) {
			changingEpic = null;
			return;
		}
		// Read before the write: a filter may hide the row once it is under its new epic.
		const away = focusAfterLeaving(s.key);
		busy = true;
		notice = null;
		leaving = null;
		let saved = false;
		try {
			await unwrap(
				api.PATCH('/api/v1/stories/{key}', { params: { path: { key: s.key } }, body: { version: s.version, epic: c.epic || null } })
			);
			changingEpic = null;
			saved = true;
			leaving = { key: s.key, apply: away };
			await reload();
		} catch (err) {
			changingEpic = null;
			await failed(err);
		} finally {
			busy = false;
		}
		// Saved: the reload that lands moves focus (the row may now sit under another epic, or be hidden by the filter).
		if (!saved) await refocus(s.key);
	}
</script>

<svelte:head><title>Backlog · Reeve</title></svelte:head>

<div class="page">
	<PageHeader title="Backlog" count={countText}>
		<BacklogFilters {epics} {initiatives} {labels}>
			<button type="button" onclick={() => open('epic')}>New epic</button>
			<button type="button" class="primary" onclick={() => open('story')}>New story</button>
		</BacklogFilters>
	</PageHeader>
	<p class="visually-hidden" aria-live="polite">{announce}</p>

	{#if !loaded}
		{#if loadError}
			<Notice tone="error" onretry={reload}>Could not load the backlog: {loadError}</Notice>
		{:else}
			<p class="muted" aria-busy="true">Loading…</p>
		{/if}
	{:else}
		<!-- One notice at a time: what the last write said wins over a failed refetch (a retry would not answer it). -->
		{#if stale && !notice}<Notice onretry={reload} ondismiss={() => (stale = null)}>{stale}</Notice>{/if}
		{#if notice}<Notice ondismiss={() => (notice = null)}>{notice}</Notice>{/if}
		{#if rows.length === 0 && (filtered || listState !== 'todo')}
			{@render noMatch()}
		{:else if rows.length === 0 && (view !== 'epic' || (epics.length === 0 && backlog.sections.length === 0))}
			<p class="muted empty">Nothing waiting</p>
		{:else if view !== 'epic'}
			{@render list(rows, true)}
		{:else}
			{#if backlog.noEpic.length}
				<section aria-label="No epic" class="section">
					<h2 class="initiative" tabindex="-1">
						No epic <span class="count">{filtered ? matches(backlog.noEpic.length) : `${backlog.untriaged} untriaged`}</span>
					</h2>
					{@render list(backlog.noEpic, false)}
				</section>
			{/if}
			{#each backlog.sections as sec (sec.key ?? '')}
				<section aria-label={sec.title} class="section">
					<h2 class="initiative" tabindex="-1">
						{#if sec.key}<a href="/initiatives/{encodeURIComponent(sec.key)}">{sec.title}</a>{:else}{sec.title}{/if}
					</h2>
					{#each sec.epics as g (g.epic.key)}
						<div class="group">
							<h3 tabindex="-1">
								<a href="/epics/{encodeURIComponent(g.epic.key)}">{g.epic.title}</a>
								<span class="key">{g.epic.key}</span>
								<span class="summary"><StatusDot tone={g.tone} label={g.summary} /></span>
							</h3>
							{#if g.rows.length}{@render list(g.rows, false)}{/if}
						</div>
					{:else}
						<p class="muted">No epics</p>
					{/each}
				</section>
			{/each}
			{#if backlog.orphans.length}
				<section aria-label="Epic not loaded" class="section">
					<h2 class="initiative" tabindex="-1">Epic not loaded</h2>
					{@render list(backlog.orphans, true)}
				</section>
			{/if}
		{/if}
		{#if searchAll && rows.length}{@render searchAllLink()}{/if}
	{/if}
</div>

{#snippet searchAllLink()}
	<p class="muted footer">
		Not here? <a href={searchAll}>Search all items, including epics, initiatives and done work, for “{text}”</a>
	</p>
{/snippet}

{#snippet noMatch()}
	<div class="empty">
		<p class="muted">
			No stories match{#if Object.keys(backlogFilters(page.url, session.me?.email)).length}
				· <button type="button" class="link" onclick={clearFilters}>Clear filters</button>{/if}
		</p>
		{#if searchAll}{@render searchAllLink()}{/if}
	</div>
{/snippet}

{#snippet list(shown: StoryWithReadiness[], showEpic: boolean)}
	<BacklogList
		rows={shown}
		full={full ?? []}
		{showEpic}
		{epicTitle}
		href={(key) => storyHref(page.url, key)}
		busy={busy || pending}
		{rankable}
		query={text}
		onrank={rank}
		onmove={startMove}
		onepic={(item) => (changingEpic = { item, epic: item.story.epic ?? '' })}
	/>
{/snippet}

{#if creating?.kind === 'story'}
	<Dialog title="New story" onclose={() => (creating = null)} closable={!formBusy}>
		<StoryForm {epics} bind:busy={formBusy} oncreated={created(creating.id)} />
	</Dialog>
{:else if creating?.kind === 'epic'}
	<Dialog title="New epic" onclose={() => (creating = null)} closable={!formBusy}>
		<NewEpicForm {initiatives} bind:busy={formBusy} oncreated={created(creating.id)} />
	</Dialog>
{/if}

{#if resolving}
	{@const r = resolving}
	<Dialog title="Move {r.item.story.key} to done" onclose={() => (resolving = null)}>
		<p>How was it resolved?</p>
		<div class="actions">
			<button type="button" onclick={() => (resolving = null)}>Cancel</button>
			{#each resolutionsFor(r.item.story.delivery, r.item.story.resolution ?? null) as res (res)}
				<button
					type="button"
					onclick={() => {
						// Built before `resolving` is cleared: `r` follows it and reads null afterwards.
						const m = { ...r, resolution: res };
						resolving = null;
						move(m);
					}}>{res}</button
				>
			{/each}
		</div>
	</Dialog>
{/if}

{#if changingEpic}
	{@const c = changingEpic}
	<Dialog title="Change the epic of {c.item.story.key}" onclose={() => (changingEpic = null)} closable={!busy}>
		<form aria-label="Change epic" onsubmit={saveEpic}>
			<label class="field">
				Epic
				<select bind:value={changingEpic.epic} disabled={busy}>
					<option value="">No epic</option>
					{#each epics as ep (ep.key)}
						<option value={ep.key}>{ep.key} {ep.title}</option>
					{/each}
					{#if c.item.story.epic && !epics.some((ep) => ep.key === c.item.story.epic)}
						<option value={c.item.story.epic}>{c.item.story.epic}</option>
					{/if}
				</select>
			</label>
			<div class="actions">
				<button type="button" onclick={() => (changingEpic = null)} disabled={busy}>Cancel</button>
				<button type="submit" class="primary" disabled={busy}>Save</button>
			</div>
		</form>
	</Dialog>
{/if}

{#if override}
	{@const o = override}
	<OverridePrompt
		storyKey={o.item.story.key}
		target={COLUMN_LABEL[o.to]}
		reasons={o.reasons}
		{busy}
		onconfirm={(why) => move(o, why)}
		oncancel={() => (override = null)}
	/>
{/if}

<style>
	.page {
		max-width: 1100px;
		margin: 0 auto;
	}
	.empty .link {
		height: auto;
		padding: 0;
		border: 0;
		background: none;
		color: var(--accent);
		font-size: inherit;
		text-decoration: underline;
		cursor: pointer;
	}
	.footer {
		margin-top: 18px;
	}
	.section + .section {
		margin-top: 24px;
	}
	.group {
		margin-top: 12px;
	}
	.initiative {
		margin: 0 0 8px;
		font-size: 13px;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--muted);
	}
	.count {
		font-weight: 400;
		text-transform: none;
		letter-spacing: 0;
	}
	h3 {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 2px 10px;
		margin: 0 0 6px;
		font-size: 16px;
	}
	.key {
		color: var(--muted);
		font-size: 13px;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 12px;
	}
	.summary {
		margin-left: auto;
		font-size: 13px;
		font-weight: 400;
		color: var(--muted);
	}
</style>
