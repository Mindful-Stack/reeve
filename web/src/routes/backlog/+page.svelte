<!-- web/src/routes/backlog/+page.svelte: the ranked todo list and the home of every epic. Readiness is the
     server's (kb/ADR-0003 R1); a refused move raises the interactive override prompt (R2). -->
<script lang="ts">
	import { tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import { storyHref } from '#lib/drawer.ts';
	import type { Epic, Initiative, Reason, StoryWithReadiness } from '#lib/api/types.ts';
	import { groupBacklog } from '#lib/backlog.ts';
	import {
		applyFilters,
		COLUMN_LABEL,
		fullLabel,
		isKind,
		KINDS,
		orderAfterDrop,
		resolutionsFor,
		type Column,
		type Resolution
	} from '#lib/board.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { session } from '#lib/me.svelte.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { moveStep, postMove } from '#lib/moves.ts';
	import { rankStory } from '#lib/rank.ts';
	import BacklogList from '#lib/components/BacklogList.svelte';
	import NewEpicForm from '#lib/components/NewEpicForm.svelte';
	import OverridePrompt from '#lib/components/OverridePrompt.svelte';
	import StoryForm from '#lib/components/StoryForm.svelte';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';

	/** The todo list as the server last reported it, in rank order. Nothing changes until a reload: no optimistic moves. */
	let todo = $state<StoryWithReadiness[] | null>(null);
	let epics = $state<Epic[]>([]);
	let initiatives = $state<Initiative[]>([]);
	let loadError = $state<string | null>(null);
	/** A refetch failed after the page had loaded: the list stays; the notice offers a retry. */
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
	/** A moved row whose focus target waits for an applied reload to remove the row (a superseded reload does not). */
	let leaving: { key: string; apply: () => void } | null = null;
	/** Read by screen readers after a rank change. */
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

	// Filters and the grouping live in the query string so they survive a reload.
	const q = $derived(page.url.searchParams);
	const ranked = $derived(q.get('group') === 'rank');
	const filtered = $derived(q.get('mine') === '1' || q.get('ready') === '1' || isKind(q.get('kind')));
	const shown = $derived.by(() => {
		if (!todo) return [];
		const kind = q.get('kind');
		const f = applyFilters(todo, {
			mine: q.get('mine') === '1' ? session.me?.email : undefined,
			kind: isKind(kind) ? kind : undefined
		});
		return q.get('ready') === '1' ? f.filter((s) => s.readiness.ready) : f;
	});
	const backlog = $derived(groupBacklog(session.config?.horizon_buckets ?? [], initiatives, epics, shown));
	const epicTitle = $derived(new Map(epics.map((e) => [e.key, e.title])));

	function set(name: string, value: string | null): Promise<void> {
		const url = new URL(page.url.href);
		if (value) url.searchParams.set(name, value);
		else url.searchParams.delete(name);
		return goto(url, { replace: true, reset: false });
	}

	function clearFilters() {
		const url = new URL(page.url.href);
		for (const name of ['mine', 'ready', 'kind']) url.searchParams.delete(name);
		goto(url, { replace: true, reset: false });
	}

	let latest = 0;

	/** A write's own reload and a live refetch can overlap; only the latest request's answer (or failure) lands. Returns
	 *  the todo list this request fetched, in rank order, even when a newer request superseded it (null on failure), so a
	 *  write can read its own result. */
	async function reload(): Promise<StoryWithReadiness[] | null> {
		const seq = ++latest;
		try {
			const [s, e, i] = await Promise.all([
				unwrap(api.GET('/api/v1/stories', { params: { query: { delivery: 'todo' } } })),
				unwrap(api.GET('/api/v1/epics')),
				unwrap(api.GET('/api/v1/initiatives'))
			]);
			const list = [...s].sort((a, b) => a.story.rank - b.story.rank);
			if (seq !== latest) return list;
			todo = list;
			if (leaving && !list.some((s) => s.story.key === leaving?.key)) {
				const l = leaving;
				leaving = null;
				// Once the row is gone from the page, and unless the user has put focus somewhere else meanwhile.
				void tick().then(() => {
					if (focusIsFree(null)) l.apply();
				});
			}
			epics = e;
			initiatives = i;
			loadError = stale = null;
			return list;
		} catch (err) {
			if (seq !== latest) return null;
			const m = errorMessage(err);
			if (todo) stale = `Could not reload the backlog: ${m}`;
			else loadError = m;
			return null;
		}
	}

	reload();
	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
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
		const full = todo;
		if (!full || busy || !orderAfterDrop(full, key, before)) return;
		notice = null;
		leaving = null;
		busy = true;
		try {
			await rankStory(full, key, before);
			// This write's own answer: a live refetch may have overtaken it, landed or not.
			const list = (await reload()) ?? todo ?? [];
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

	/** Where focus goes once `key`'s row leaves the list (read from the page before the reload): the next row's actions
	 *  trigger in the same list, else the previous row's, else the group's heading, else the page heading. `reload` applies
	 *  it once an applied answer no longer has the row. */
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
		if (busy) return;
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
		// The override prompt takes focus while it is open; a moved row's focus waits for the reload that removes it.
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
		busy = true;
		notice = null;
		try {
			await unwrap(
				api.PATCH('/api/v1/stories/{key}', { params: { path: { key: s.key } }, body: { version: s.version, epic: c.epic || null } })
			);
			changingEpic = null;
			await reload();
		} catch (err) {
			changingEpic = null;
			await failed(err);
		} finally {
			busy = false;
		}
		// The row may now sit under another epic: focus its actions trigger wherever it is.
		await refocus(s.key);
	}
</script>

<svelte:head><title>Backlog · Reeve</title></svelte:head>

<div class="page">
	<PageHeader title="Backlog" count={todo?.length}>
		<Toggle
			label="Group"
			options={[
				{ value: 'epic', label: 'By epic' },
				{ value: 'rank', label: 'Ranked' }
			]}
			value={ranked ? 'rank' : 'epic'}
			onchange={(v) => set('group', v === 'rank' ? 'rank' : null)}
		/>
		<label class="check">
			<input type="checkbox" checked={q.get('mine') === '1'} onchange={(e) => set('mine', e.currentTarget.checked ? '1' : null)} />
			Mine
		</label>
		<label class="check">
			<input type="checkbox" checked={q.get('ready') === '1'} onchange={(e) => set('ready', e.currentTarget.checked ? '1' : null)} />
			Ready only
		</label>
		<label class="field inline">
			Kind
			<select value={isKind(q.get('kind')) ? q.get('kind') : ''} onchange={(e) => set('kind', e.currentTarget.value || null)}>
				<option value="">any</option>
				{#each KINDS as k (k)}<option value={k}>{k}</option>{/each}
			</select>
		</label>
		<span class="spacer"></span>
		<button type="button" onclick={() => open('epic')}>New epic</button>
		<button type="button" class="primary" onclick={() => open('story')}>New story</button>
	</PageHeader>
	<p class="visually-hidden" aria-live="polite">{announce}</p>

	{#if loadError}
		<Notice tone="error" onretry={reload}>Could not load the backlog: {loadError}</Notice>
	{:else if !todo}
		<p class="muted" aria-busy="true">Loading…</p>
	{:else}
		<!-- One notice at a time: what the last write said wins over a failed refetch (a retry would not answer it). -->
		{#if stale && !notice}<Notice onretry={reload} ondismiss={() => (stale = null)}>{stale}</Notice>{/if}
		{#if notice}<Notice ondismiss={() => (notice = null)}>{notice}</Notice>{/if}
		{#if shown.length === 0 && (ranked || (epics.length === 0 && backlog.sections.length === 0))}
			{#if todo.length && filtered}{@render noMatch()}{:else}<p class="muted empty">Nothing waiting</p>{/if}
		{:else if ranked}
			{@render list(shown, true)}
		{:else}
			{#if shown.length === 0 && todo.length && filtered}{@render noMatch()}{/if}
			{#if backlog.noEpic.length}
				<section aria-label="No epic" class="section">
					<h2 class="initiative" tabindex="-1">No epic <span class="count">{backlog.untriaged} untriaged</span></h2>
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
	{/if}
</div>

{#snippet noMatch()}
	<p class="muted empty">
		Nothing matches these filters · <button type="button" class="link" onclick={clearFilters}>Clear filters</button>
	</p>
{/snippet}

{#snippet list(rows: StoryWithReadiness[], showEpic: boolean)}
	<BacklogList
		{rows}
		full={todo ?? []}
		{showEpic}
		{epicTitle}
		href={(key) => storyHref(page.url, key)}
		{busy}
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
	}
	.spacer {
		flex: 1;
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
