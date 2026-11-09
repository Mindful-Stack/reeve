<script lang="ts">
	import { tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import type { Initiative, RequiredDecision } from '#lib/api/types.ts';
	import InitiativeCard from '#lib/components/InitiativeCard.svelte';
	import MarkUrgentDialog from '#lib/components/MarkUrgentDialog.svelte';
	import WarRoom from '#lib/components/WarRoom.svelte';
	import { onLive } from '#lib/events.svelte.ts';
	import { sameEmail } from '#lib/decisions.ts';
	import { initiativeCreate, type InitiativeDraft } from '#lib/forms.ts';
	import { CreateCycle } from '#lib/idempotency.ts';
	import { session } from '#lib/me.svelte.ts';
	import { blockingByItem, bucketise, horizonLayout } from '#lib/planning.ts';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';

	let initiatives = $state<Initiative[] | null>(null);
	let decisions = $state<RequiredDecision[]>([]);
	let notice = $state<string | null>(null);
	let busy = $state(false);
	let over = $state<string | null>(null);

	let creating = $state(false);
	let draft = $state<InitiativeDraft>({ title: '', horizon: '', target_month: '', brief: '' });
	/** The form's sends: its key survives cancel and reopen and is replaced only after a successful create. */
	const cycle = new CreateCycle();
	let createError = $state<string | null>(null);
	let newButton: HTMLButtonElement | undefined;
	let lastToggle: HTMLButtonElement | undefined = $state();

	/** All initiatives, or only the signed-in user's (`?mine=1`, so it survives a reload). */
	const scope = $derived<'all' | 'mine'>(page.url.searchParams.get('mine') === '1' ? 'mine' : 'all');
	/** The last configured horizon is collapsed under the columns until this is set. */
	let showLast = $state(false);

	const configured = $derived(session.config?.horizon_buckets ?? []);
	const shown = $derived(
		scope === 'mine' ? (initiatives ?? []).filter((i) => !!i.owner && !!session.me && sameEmail(i.owner, session.me.email)) : (initiatives ?? [])
	);
	const buckets = $derived(bucketise(configured, shown));
	/** `layout.columns` are the rendered columns (the last horizon among them when expanded): only these are drop targets. */
	const layout = $derived(horizonLayout(configured, buckets, showLast));
	const columns = $derived(layout.columns);
	const lastId = 'planning-last-horizon';
	const blocking = $derived(blockingByItem(decisions));

	let loadError = $state<string | null>(null);
	let latest = 0;

	/** Every write is followed by this, so a card's `version` is always the one the server last returned. A write's reload
	 *  and a live refetch can overlap: only the latest request's answer (or failure) lands, a superseded one is dropped. */
	async function reload() {
		const seq = ++latest;
		let i: Initiative[], d: RequiredDecision[];
		try {
			[i, d] = await Promise.all([unwrap(api.GET('/api/v1/initiatives')), unwrap(api.GET('/api/v1/decisions'))]);
		} catch (e) {
			if (seq !== latest) return;
			throw e;
		}
		if (seq !== latest) return;
		initiatives = i;
		decisions = d;
		loadError = null;
	}

	function show(e: unknown): string {
		if (e instanceof Refused) return `Refused: ${e.reasons.map((r) => r.reason.replaceAll('_', ' ')).join(', ')}`;
		if (e instanceof Conflict) return 'reloaded, try again';
		if (e instanceof ApiError) return e.message;
		return errorMessage(e);
	}

	// A failed first load shows the error until any later reload (a write or a live refetch) succeeds.
	retry();
	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
	// A failed refetch keeps the plan on screen and shows the load notice, with Retry and Dismiss.
	onLive(retry);

	function setScope(v: 'all' | 'mine'): Promise<void> {
		const url = new URL(page.url.href);
		if (v === 'mine') url.searchParams.set('mine', '1');
		else url.searchParams.delete('mine');
		return goto(url, { replace: true, reset: false });
	}

	/** The initiative whose "Mark urgent" dialog is open. */
	let marking = $state<string | null>(null);
	let strip: { reload: () => Promise<void> } | undefined = $state();
	const markingReason = $derived(initiatives?.find((i) => i.key === marking)?.urgent_mark?.reason ?? null);
	let markError = $state<string | null>(null);

	async function markUrgent(reason: string | null) {
		const key = marking;
		if (!key || busy) return;
		busy = true;
		markError = null;
		try {
			await unwrap(api.POST('/api/v1/initiatives/{key}/urgent', { params: { path: { key } }, body: { reason } }));
			marking = null;
		} catch (e) {
			markError = show(e);
		}
		try {
			await Promise.all([reload(), strip?.reload()]);
		} catch (e) {
			loadError = show(e);
		} finally {
			busy = false;
		}
	}

	async function clearUrgent(key: string) {
		if (busy) return;
		busy = true;
		notice = null;
		try {
			await unwrap(api.DELETE('/api/v1/initiatives/{key}/urgent', { params: { path: { key } } }));
		} catch (e) {
			notice = show(e);
		}
		try {
			await Promise.all([reload(), strip?.reload()]);
		} catch (e) {
			loadError = show(e);
		} finally {
			busy = false;
		}
		await refocus(key);
	}

	async function move(key: string, horizon: string) {
		// Read the version from the latest load at drop time, never from a value captured at an earlier render.
		const current = initiatives?.find((i) => i.key === key);
		if (!current || current.horizon === horizon || busy || !configured.includes(horizon)) return;
		busy = true;
		notice = null;
		try {
			await unwrap(
				api.PATCH('/api/v1/initiatives/{key}', {
					params: { path: { key } },
					body: { version: current.version, horizon }
				})
			);
		} catch (e) {
			notice = show(e);
		}
		try {
			await reload();
		} catch (e) {
			loadError = show(e);
		} finally {
			busy = false;
		}
		await refocus(key);
	}

	/** After a move and its reload, focus the moved card's actions menu trigger (its menu is gone), else the card; a card
	 *  now in the collapsed last horizon is not rendered, so focus goes to that horizon's toggle. */
	async function refocus(key: string) {
		await tick();
		const card = [...document.querySelectorAll<HTMLElement>('[data-testid^="card-"]')].find(
			(el) => el.dataset.testid === `card-${key}`
		);
		(card?.querySelector<HTMLElement>('[data-control="menu"]') ?? card ?? lastToggle)?.focus();
	}

	async function toggleLast() {
		showLast = !showLast;
		if (!showLast) return;
		await tick();
		document.getElementById(lastId)?.scrollIntoView?.({ inline: 'nearest', block: 'nearest' });
	}

	function startCreate() {
		if (!draft.horizon) draft.horizon = configured[0] ?? '';
		createError = null;
		creating = true;
	}

	function retry() {
		reload().catch((e: unknown) => (loadError = show(e)));
	}

	function cancelCreate() {
		if (busy) return; // a pending create keeps its dialog: its completion navigates or shows the error here
		creating = false;
		createError = null;
		void tick().then(() => newButton?.focus());
	}

	async function createInitiative(e: SubmitEvent) {
		e.preventDefault();
		if (busy) return;
		const p = initiativeCreate(draft, configured);
		if (!p.ok) {
			createError = p.error;
			return;
		}
		busy = true;
		createError = null;
		const out = await cycle.send(p.text, (idempotency_key) =>
			unwrap(api.POST('/api/v1/initiatives', { body: { ...p.body, idempotency_key } }))
		);
		busy = false;
		if (out.kind === 'sent') {
			draft = { title: '', horizon: configured[0] ?? '', target_month: '', brief: '' };
			creating = false;
			await goto(`/initiatives/${encodeURIComponent(out.value.key)}`);
		} else if (out.kind === 'failed') createError = show(out.error); // the draft and its key stay for a retry
	}

	function ondragover(e: DragEvent, bucket: string) {
		if (busy || !configured.includes(bucket)) return; // the Other column is not a horizon: nothing can be moved there
		e.preventDefault();
		if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
		over = bucket;
	}

	function ondrop(e: DragEvent, bucket: string) {
		e.preventDefault();
		over = null;
		const key = e.dataTransfer?.getData('text/plain');
		if (key) void move(key, bucket);
	}
</script>

<svelte:head><title>Planning · Reeve</title></svelte:head>

<PageHeader title="Planning">
	<Toggle
		label="Show"
		options={[
			{ value: 'all', label: 'All' },
			{ value: 'mine', label: 'Mine' }
		]}
		value={scope}
		onchange={setScope}
	/>
	<button type="button" class="primary new" bind:this={newButton} onclick={startCreate}>New initiative</button>
</PageHeader>

{#if creating}
	<Dialog title="New initiative" onclose={cancelCreate} closable={!busy}>
		<form class="create" aria-label="New initiative" onsubmit={createInitiative}>
			<label class="field">Title <input bind:value={draft.title} required disabled={busy} /></label>
			<label class="field">
				Horizon
				<select bind:value={draft.horizon} disabled={busy}>
					{#each configured as b (b)}<option value={b}>{b}</option>{/each}
				</select>
			</label>
			<label class="field">Target month <input bind:value={draft.target_month} placeholder="YYYY-MM" disabled={busy} /></label>
			<label class="field">Brief (Markdown) <textarea bind:value={draft.brief} rows="4" disabled={busy}></textarea></label>
			{#if createError}<p class="error" role="alert">{createError}</p>{/if}
			<div class="actions">
				<button type="button" onclick={cancelCreate} disabled={busy}>Cancel</button>
				<button type="submit" class="primary" disabled={busy || !draft.title.trim()}>Create initiative</button>
			</div>
		</form>
	</Dialog>
{/if}

{#if notice}<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>{/if}

{#if loadError}
	<!-- A failed refetch keeps the loaded plan: Retry or Dismiss. A failed first load has nothing to fall back to: Retry only. -->
	<Notice tone="error" onretry={retry} ondismiss={initiatives ? () => (loadError = null) : undefined}
		>Could not load the plan: {loadError}</Notice
	>
{/if}

<!-- Outside the Mine filter: it loads its own data and lists every mark. -->
<WarRoom
	bind:this={strip}
	onchange={retry}
	onempty={() => newButton?.focus()}
/>

{#if marking}
	<MarkUrgentDialog
		initial={markingReason}
		{busy}
		error={markError}
		onsubmit={markUrgent}
		oncancel={() => {
			if (!busy) marking = null;
		}}
	/>
{/if}

{#if !initiatives}
	{#if !loadError}
		<!-- The first load keeps the column frames, each saying it is loading. -->
		<div class="columns" aria-busy="true">
			{#each layout.columns as bucket (bucket)}
				<section class="column" aria-label={bucket}>
					<h2>{bucket}</h2>
					<p class="muted">Loading…</p>
				</section>
			{/each}
		</div>
	{/if}
{:else}
	<div class="columns" aria-busy={busy}>
		{#each columns as bucket (bucket)}
			{@const items = buckets.get(bucket) ?? []}
			<section
				class="column"
				id={bucket === layout.collapsed ? lastId : undefined}
				class:over={over === bucket}
				aria-label={bucket}
				ondragover={(e) => ondragover(e, bucket)}
				ondragleave={() => (over = over === bucket ? null : over)}
				ondrop={(e) => ondrop(e, bucket)}
			>
				<h2>{bucket} <span class="count">{items.length}</span></h2>
				{#if !configured.includes(bucket)}
					<p class="muted hint">Horizons no longer configured. Move an initiative to a horizon to replan it.</p>
				{/if}
				{#each items as initiative (initiative.key)}
					<InitiativeCard
						{initiative}
						decisions={blocking.get(initiative.key)}
						buckets={configured}
						{busy}
						onmove={move}
						onmark={(key) => ((markError = null), (marking = key))}
						onclear={clearUrgent}
					/>
				{:else}
					<p class="muted">No initiatives</p>
				{/each}
			</section>
		{/each}
	</div>
	{#if layout.collapsed !== null}
		<button
			type="button"
			class="ghost collapsed"
			bind:this={lastToggle}
			aria-expanded={showLast}
			aria-controls={showLast ? lastId : undefined}
			onclick={toggleLast}
			>{layout.collapsed} · {buckets.get(layout.collapsed)?.length ?? 0}</button
		>
	{/if}
{/if}

<style>
	.new {
		margin-left: auto;
	}
	.columns {
		display: grid;
		grid-auto-flow: column;
		grid-auto-columns: minmax(var(--column-min), 1fr);
		gap: 12px;
		overflow-x: auto;
		scroll-snap-type: x proximity;
		align-items: start;
	}
	.column {
		display: grid;
		gap: 8px;
		align-content: start;
		min-height: 8rem;
		scroll-snap-align: start;
	}
	.column.over {
		outline: 2px dashed var(--accent);
		outline-offset: 4px;
	}
	.column p {
		margin: 0;
		font-size: 14px;
	}
	.hint {
		font-size: 13px;
	}
	.collapsed {
		margin-top: 16px;
	}
	.create {
		display: grid;
		gap: 12px;
		min-width: min(28rem, 80vw);
	}
	.create .actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}
	.error {
		margin: 0;
		color: var(--danger);
	}
	[aria-busy='true'] {
		opacity: 0.7;
	}
</style>
