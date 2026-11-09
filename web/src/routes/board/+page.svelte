<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import { storyHref } from '#lib/drawer.ts';
	import type { Epic, Reason, Sprints, StoryWithReadiness } from '#lib/api/types.ts';
	import {
		accepts,
		afterPart,
		applyFilters,
		COLUMN_LABEL,
		columnHeading,
		COLUMNS,
		columnise,
		dropBefore,
		fullLabel,
		isKind,
		KINDS,
		neighbourBefore,
		orderAfterDrop,
		resolutionsFor,
		splitUpNext,
		type Column,
		type Resolution
	} from '#lib/board.ts';
	import { session } from '#lib/me.svelte.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { moveStep, postMove } from '#lib/moves.ts';
	import { rankStory } from '#lib/rank.ts';
	import { goalFilter, nudgeStillDue, shouldNudge } from '#lib/sprint.ts';
	import GoalsStrip from '#lib/components/GoalsStrip.svelte';
	import NudgeBar from '#lib/components/NudgeBar.svelte';
	import OverridePrompt from '#lib/components/OverridePrompt.svelte';
	import StoryCard from '#lib/components/StoryCard.svelte';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';

	type Move = { item: StoryWithReadiness; to: Column; resolution?: Resolution };

	// The whole board as the server last reported it. Nothing here changes until a reload: no optimistic moves.
	let columns = $state<Record<Column, StoryWithReadiness[]> | null>(null);
	let epics = $state<Epic[]>([]);
	/** The sprints as the server last reported them; only the active one shows here (the strip, the nudge). */
	let sprints = $state<Sprints | null>(null);
	const active = $derived(sprints?.active ?? null);
	/** The card that just started serving no goal (the move's reply said so), asking which goal it serves. */
	let nudge = $state<{ key: string } | null>(null);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);
	let dragging = $state<{ item: StoryWithReadiness; from: Column } | null>(null);
	/** A drop into done waiting for its resolution. */
	let resolving = $state<Move | null>(null);
	/** A move refused for readiness reasons, waiting for an override reason. */
	let override = $state<(Move & { reasons: Reason[] }) | null>(null);

	const epicTitle = $derived(new Map(epics.map((e) => [e.key, e.title])));

	// Filters live in the query string so they survive a reload.
	const query = $derived(page.url.searchParams);
	const filters = $derived.by(() => {
		const kind = query.get('kind');
		return {
			// Only a goal of the active sprint: without one the strip is gone, and a stale `?goal=` must not hide every card.
			goal: goalFilter(query.get('goal'), active ? active.sprint.goals.map((g) => g.id) : null),
			mine: query.get('mine') === '1' ? session.me?.email : undefined,
			epic: query.get('epic') || undefined,
			kind: isKind(kind) ? kind : undefined,
			untriaged: query.get('untriaged') === '1'
		};
	});
	const visible = $derived.by(() => {
		if (!columns) return null;
		const cols = columns;
		return Object.fromEntries(COLUMNS.map((c) => [c, applyFilters(cols[c], filters)])) as Record<
			Column,
			StoryWithReadiness[]
		>;
	});
	/** Up next: the shown todo cards, ready first (the server's readiness, kb/ADR-0003 R1). */
	const upNext = $derived(visible ? splitUpNext(visible.todo) : null);
	/** The "Not ready" disclosure is open when the query string says so, so it survives a reload. */
	const showNotReady = $derived(query.get('notready') === '1');

	function setFilter(name: string, value: string | null): Promise<void> {
		const url = new URL(page.url.href);
		if (value) url.searchParams.set(name, value);
		else url.searchParams.delete(name);
		return goto(url, { replace: true, reset: false });
	}

	/** "Show not ready" under "Nothing ready": opens the disclosure, then focuses its summary (the button goes away). */
	async function showNotReadyPart() {
		await setFilter('notready', '1');
		await tick();
		document.querySelector<HTMLElement>('details.not-ready > summary')?.focus();
	}

	let latest = 0;

	/** A write's own reload and a live refetch can overlap; only the latest request's answer (or failure) lands, so an
	 *  older response arriving last never puts a moved card back in its old column. */
	async function reload() {
		const seq = ++latest;
		try {
			const [stories, list, sprintList] = await Promise.all([
				unwrap(api.GET('/api/v1/stories', { params: { query: { done: 'recent' } } })),
				unwrap(api.GET('/api/v1/epics')),
				// The sprints are an extra: a failure leaves the board without its strip and nudge, never without its cards.
				unwrap(api.GET('/api/v1/sprints')).catch(() => null)
			]);
			if (seq !== latest) return;
			columns = columnise(stories);
			epics = list;
			sprints = sprintList;
			// The bar goes once its story was linked (here or elsewhere), moved out of active work, or left the board.
			if (nudge) {
				const key = nudge.key;
				const now = stories.find((s) => s.story.key === key);
				if (!now || !nudgeStillDue(now.story)) nudge = null;
			}
			loadError = null;
		} catch (e) {
			if (seq !== latest) return;
			loadError = errorMessage(e);
		}
	}

	onMount(reload);
	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
	onLive(reload);

	/** Shows what went wrong for anything but a refusal; a conflict also reloads the board. */
	async function failed(e: unknown) {
		if (e instanceof Conflict) {
			notice = 'Someone else changed this story; the board was reloaded.';
			await reload();
		} else if (e instanceof ApiError) {
			notice = e.message;
		} else {
			notice = errorMessage(e);
		}
	}

	/** After a write and its reload, focus the moved card's actions menu trigger (its items are gone once the menu
	 *  closes), or the card itself when it has no menu. */
	async function refocus(key: string) {
		await tick();
		let card = findCard(key);
		// A not-ready story moved back to todo lands in the Not ready disclosure: open it first, or the trigger is hidden.
		if (card?.closest('details.not-ready:not([open])')) {
			await setFilter('notready', '1');
			await tick();
			card = findCard(key);
		}
		(card?.querySelector<HTMLElement>('[data-control="menu"]') ?? card)?.focus();
	}

	/** The card for `key`, compared by dataset (no CSS.escape, which jsdom lacks). */
	function findCard(key: string): HTMLElement | undefined {
		return [...document.querySelectorAll<HTMLElement>('[data-testid^="card-"]')].find(
			(el) => el.dataset.testid === `card-${key}`
		);
	}

	async function move(m: Move, overrideReason?: string) {
		const s = m.item.story;
		busy = true;
		try {
			const out = await postMove(s, m.to, { resolution: m.resolution, overrideReason });
			if (out.kind === 'override') {
				override = { ...m, reasons: out.reasons };
			} else {
				override = null;
				if (out.kind === 'notice') notice = out.notice;
				else {
					notice = null;
					// Keyed on the reply's story, never the card before the move: the server says which goals it serves now.
					if (shouldNudge(s.delivery, out.story, !!active)) nudge = { key: s.key };
					await reload();
				}
			}
		} catch (e) {
			override = null;
			await failed(e);
		} finally {
			busy = false;
		}
		// The override prompt takes focus while it is open; otherwise focus follows the card.
		if (!override) await refocus(s.key);
	}

	/** Every move goes through here, from a drop or from a card control. A move into done (including a re-resolution
	 *  within done) first asks for the resolution; a done story with nothing to re-resolve to does nothing. */
	function startMove(item: StoryWithReadiness, to: Column) {
		if (busy) return;
		notice = null;
		nudge = null;
		const step = moveStep(item.story, to);
		if (step === 'resolve') resolving = { item, to };
		else if (step === 'move') move({ item, to });
	}

	/** Ranks against the complete todo column (`rankStory`); a placement that changes nothing sends nothing and leaves the
	 *  notice and focus alone. */
	async function rank(movedKey: string, beforeKey: string | null) {
		if (!columns) return;
		const full = columns.todo;
		if (!orderAfterDrop(full, movedKey, beforeKey)) return;
		notice = null;
		busy = true;
		try {
			await rankStory(full, movedKey, beforeKey);
			await reload();
		} catch (e) {
			if (e instanceof Refused) notice = e.reasons.map((r) => fullLabel(r)).join('; ');
			else await failed(e);
			if (!(e instanceof Conflict)) await reload();
		} finally {
			busy = false;
		}
		await refocus(movedKey);
	}

	/** Move up / Move down on a ready up-next card: the target comes from the shown ready cards; the server places the
	 *  card in the full todo column. */
	function rankHandler(key: string, direction: 'up' | 'down'): (() => void) | null {
		const target = upNext ? neighbourBefore(upNext.ready, key, direction) : null;
		if (!target) return null;
		const ready = upNext?.ready ?? [];
		// Past the last ready card: before the story that follows the ready part, so the card stays in its part.
		return () => rank(key, target.before ?? (columns ? afterPart(columns.todo, ready) : null));
	}

	/** The nudge's goal: one link write (`POST /goals/{id}/links`), separate from the move, which has already landed. */
	async function linkGoal(key: string, goalId: number) {
		if (busy) return;
		busy = true;
		notice = null;
		try {
			await unwrap(api.POST('/api/v1/goals/{id}/links', { params: { path: { id: goalId } }, body: { items: [key] } }));
			nudge = null;
			await reload();
		} catch (e) {
			await failed(e);
		} finally {
			busy = false;
		}
		// The bar's buttons left the DOM: focus follows the card.
		if (!nudge) await refocus(key);
	}

	/** Opening or closing "Not ready" writes `?notready=1`; a toggle that already matches the URL (the initial render
	 *  with `open` set) writes nothing. */
	function toggleNotReady(e: Event & { currentTarget: HTMLDetailsElement }) {
		const open = e.currentTarget.open;
		if (open !== showNotReady) setFilter('notready', open ? '1' : null);
	}

	function dragStart(e: DragEvent, item: StoryWithReadiness, from: Column) {
		if (busy) {
			e.preventDefault();
			return;
		}
		dragging = { item, from };
		e.dataTransfer?.setData('text/plain', item.story.key);
		if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
	}

	function dragOver(e: DragEvent, to: Column) {
		const d = dragging;
		if (!d || !accepts(d.from, to, d.item.story.resolution ?? null)) return;
		e.preventDefault();
		if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
	}

	/** A drop anywhere on a column: a move to another column, a re-resolution within done, or a reorder within todo. A
	 *  todo reorder lands where the pointer is among the shown cards (gaps and the header included), not at the end. */
	function dropOnColumn(e: DragEvent, to: Column) {
		e.preventDefault();
		const d = dragging;
		dragging = null;
		if (!d) return;
		if (d.from !== to || to === 'done') return startMove(d.item, to);
		if (to !== 'todo' || !upNext) return;
		// Up next shows two parts; a drop lands among the shown cards of the dragged card's own part only, and a drop that
		// leaves that part's order unchanged sends nothing.
		const ready = d.item.readiness.ready;
		const part = ready ? upNext.ready : upNext.notReady;
		const list = (e.currentTarget as HTMLElement).querySelector<HTMLElement>(ready ? 'ol.ready' : 'ol.not-ready-list');
		const boxes = [...(list?.querySelectorAll<HTMLElement>(':scope > li[data-key]') ?? [])].map((li) => {
			const r = li.getBoundingClientRect();
			return { key: li.dataset.key ?? '', top: r.top, height: r.height };
		});
		const before = dropBefore(e.clientY, boxes);
		if (!orderAfterDrop(part, d.item.story.key, before)) return;
		// Below the part's last card: before the story that follows the part, so the card stays in its part.
		rank(d.item.story.key, before ?? (columns ? afterPart(columns.todo, part) : null));
	}

</script>

<svelte:head><title>Board · Reeve</title></svelte:head>

<PageHeader title="Board">
	<Toggle
		label="Whose"
		options={[
			{ value: 'all', label: 'All' },
			{ value: 'mine', label: 'Mine' }
		]}
		value={query.get('mine') === '1' ? 'mine' : 'all'}
		onchange={(v) => setFilter('mine', v === 'mine' ? '1' : null)}
	/>
	<label class="field inline">
		Epic
		<select value={query.get('epic') ?? ''} onchange={(e) => setFilter('epic', e.currentTarget.value || null)}>
			<option value="">any</option>
			{#each epics as e (e.key)}
				<option value={e.key}>{e.key} {e.title}</option>
			{/each}
		</select>
	</label>
	<label class="field inline">
		Kind
		<select value={filters.kind ?? ''} onchange={(e) => setFilter('kind', e.currentTarget.value || null)}>
			<option value="">any</option>
			{#each KINDS as k (k)}
				<option value={k}>{k}</option>
			{/each}
		</select>
	</label>
	<label class="check">
		<input
			type="checkbox"
			checked={query.get('untriaged') === '1'}
			onchange={(e) => setFilter('untriaged', e.currentTarget.checked ? '1' : null)}
		/>
		Untriaged
	</label>
</PageHeader>

{#if notice}<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>{/if}

<GoalsStrip view={active} selected={filters.goal} onselect={(g) => setFilter('goal', g === null ? null : String(g))} />

{#if loadError}
	<!-- Above a loaded board the notice can be dismissed; a failed first load has nothing to fall back to, so Retry only. -->
	<Notice tone="error" onretry={reload} ondismiss={visible ? () => (loadError = null) : undefined}
		>Could not load the board: {loadError}</Notice
	>
{/if}

{#if !visible || !upNext}
	{#if !loadError}
		<!-- The first load keeps the column frames, each saying it is loading. -->
		<div class="board" aria-busy="true">
			{#each COLUMNS as col (col)}
				<section class="column" data-testid="column-{col}" aria-label={COLUMN_LABEL[col]}>
					<h2>{columnHeading(col, session.config?.done_window_days)}</h2>
					<p class="empty">Loading…</p>
				</section>
			{/each}
		</div>
	{/if}
{:else}
	<div class="board" aria-busy={busy}>
		{#each COLUMNS as col (col)}
			<section
				class="column"
				data-testid="column-{col}"
				aria-label={COLUMN_LABEL[col]}
				ondragover={(e) => dragOver(e, col)}
				ondrop={(e) => dropOnColumn(e, col)}
			>
				<h2>{columnHeading(col, session.config?.done_window_days)} <span class="count">{visible[col].length}</span></h2>
				{#if col === 'todo'}
					<ol class="ready">
						{#each upNext.ready as item (item.story.key)}
							<li data-key={item.story.key}>
								{@render card(item, col, true)}
							</li>
						{/each}
					</ol>
					{#if upNext.ready.length === 0}
						<p class="empty">
							Nothing ready{#if upNext.notReady.length && !showNotReady}{' '}<button
									type="button"
									class="link"
									onclick={showNotReadyPart}>Show not ready</button
								>{/if}
						</p>
					{/if}
					{#if upNext.notReady.length}
						<details class="not-ready" open={showNotReady} ontoggle={toggleNotReady}>
							<summary>Not ready · {upNext.notReady.length}</summary>
							<ol class="not-ready-list">
								{#each upNext.notReady as item (item.story.key)}
									<li data-key={item.story.key}>
										{@render card(item, col, false)}
									</li>
								{/each}
							</ol>
						</details>
					{/if}
				{:else}
					<ol>
						{#each visible[col] as item (item.story.key)}
							<li data-key={item.story.key}>
								{@render card(item, col, false)}
							</li>
						{/each}
					</ol>
				{/if}
			</section>
		{/each}
	</div>
{/if}

{#snippet card(item: StoryWithReadiness, col: Column, ranked: boolean)}
	<StoryCard
		{item}
		epicTitle={item.story.epic ? (epicTitle.get(item.story.epic) ?? item.story.epic) : undefined}
		href={storyHref(page.url, item.story.key)}
		ondragstart={(e) => dragStart(e, item, col)}
		ondragend={() => (dragging = null)}
		{busy}
		onmove={(to) => startMove(item, to)}
		onup={ranked ? rankHandler(item.story.key, 'up') : undefined}
		ondown={ranked ? rankHandler(item.story.key, 'down') : undefined}
		onresolve={col === 'done' && resolutionsFor('done', item.story.resolution ?? null).length > 0
			? () => startMove(item, 'done')
			: undefined}
	/>
	{#if active && nudge?.key === item.story.key}
		{@const key = item.story.key}
		<NudgeBar goals={active.sprint.goals} {busy} onpick={(id) => linkGoal(key, id)} ondismiss={() => (nudge = null)} />
	{/if}
{/snippet}

{#if resolving}
	{@const r = resolving}
	<Dialog
		title={r.item.story.delivery === 'done' ? `Change the resolution of ${r.item.story.key}` : `Move ${r.item.story.key} to done`}
		onclose={() => (resolving = null)}
	>
		{#if r.item.story.delivery === 'done'}
			<p>It is {r.item.story.resolution}. A dropped story cannot become completed.</p>
		{:else}
			<p>How was it resolved?</p>
		{/if}
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
	.inline {
		display: flex;
		align-items: center;
		gap: 6px;
	}
	.check {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		color: var(--muted);
	}
	.board {
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: minmax(var(--column-min), 1fr);
		gap: 10px;
		overflow-x: auto;
		scroll-snap-type: x proximity;
	}
	/* Every grid here holds one track capped at the column's width: an implicit auto track grows to its widest card's
	   min-content (a nowrap epic or goal, a long word), and one such card would push the whole column past its edge. */
	.column {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		align-content: start;
		gap: 8px;
		min-height: 12rem;
		padding: 8px;
		border-radius: 6px;
		background: var(--lighter-background);
		scroll-snap-align: start;
	}
	.column h2 {
		margin: 0;
		font-size: 15px;
		text-transform: none;
	}
	.empty {
		margin: 0;
		color: var(--muted);
		font-size: 14px;
	}
	summary {
		color: var(--muted);
		font-size: 14px;
		cursor: pointer;
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
	.not-ready ol {
		margin-top: 8px;
	}
	ol {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 8px;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.5rem;
	}
</style>
