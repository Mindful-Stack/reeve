<!-- The war room strip: every urgent mark on one line each, active first. Loads its own data, so the planning page's
     "Mine" filter does not hide it. Clearing and renewing in bulk ask inline, never through a browser dialog. -->
<script lang="ts">
	import { tick } from 'svelte';
	import { page } from '$app/state';
	import { api, ApiError, Refused, unwrap } from '#lib/api/client.ts';
	import type { ItemRef, WarRoomEntry } from '#lib/api/types.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { errorMessage } from '#lib/format.ts';
	import { itemRoute, ownMarkText } from '#lib/urgency.ts';
	import MarkUrgentDialog from './MarkUrgentDialog.svelte';

	let {
		clock = () => new Date(),
		onchange,
		onempty
	}: {
		/** The time source; tests pass a fixed one. Read whenever a list is applied, so "expires in" stays current. */
		clock?: () => Date;
		/** Called after a write of the strip's own landed, so the page can refresh its cards. */
		onchange?: () => void;
		/** Called when a write of the strip's own emptied it, so the page can move focus out of the vanished strip. */
		onempty?: () => void;
	} = $props();

	let entries = $state<WarRoomEntry[]>([]);
	// svelte-ignore state_referenced_locally
	let at = $state(clock());
	/** The bulk action being confirmed with the refs captured when it opened: what is confirmed is what is sent. */
	let confirm = $state<{ kind: 'clear' | 'renew'; items: ItemRef[] } | null>(null);
	let remarking = $state<WarRoomEntry | null>(null);
	let dialogError = $state<string | null>(null);
	let writeError = $state<string | null>(null);
	let busy = $state(false);
	let latest = 0;
	let heading: HTMLElement | undefined = $state();

	/** An expired mark that can be renewed: never a done story's (the drawer offers no mark on a done story either). */
	const renewable = (e: WarRoomEntry) => !e.mark.active && !e.mark.done;
	const renewables = $derived(entries.filter(renewable));
	const word = (n: number) => `${n} ${n === 1 ? 'mark' : 'marks'}`;
	const same = (a: ItemRef, b: ItemRef) => a.item === b.item && a.key === b.key;

	function show(e: unknown): string {
		if (e instanceof Refused) return `Refused: ${e.reasons.map((r) => r.reason.replaceAll('_', ' ')).join(', ')}`;
		if (e instanceof ApiError) return e.message;
		return errorMessage(e);
	}

	function apply(list: WarRoomEntry[]) {
		entries = list;
		at = clock();
		if (confirm) {
			// Every confirmed ref must still be listed, and for a renewal still be renewable (renewed elsewhere otherwise).
			const pool = confirm.kind === 'renew' ? list.filter(renewable) : list;
			if (!confirm.items.every((r) => pool.some((e) => same(e.item, r)))) confirm = null;
		}
	}

	/** Refetches the list. A failed load keeps what is shown; the page's own notice reports load failures. */
	export async function reload() {
		const seq = ++latest;
		try {
			const list = await unwrap(api.GET('/api/v1/war-room'));
			if (seq === latest) apply(list);
		} catch {
			/* keep the list on screen */
		}
	}
	void reload();
	onLive(reload);

	/** A write that answers with the refreshed list (or nothing). Returns the failure text, or null on success. A failed
	 *  write reloads too, since the list may have changed under it. `ran` is false when another write is in flight. */
	async function run(write: () => Promise<WarRoomEntry[] | unknown>): Promise<{ ran: boolean; error: string | null }> {
		if (busy) return { ran: false, error: null };
		busy = true;
		writeError = null;
		try {
			const answer = await write();
			if (Array.isArray(answer)) {
				latest++;
				apply(answer as WarRoomEntry[]);
			} else await reload();
			onchange?.();
			return { ran: true, error: null };
		} catch (e) {
			await reload();
			return { ran: true, error: show(e) };
		} finally {
			busy = false;
		}
	}

	/** Moves focus after a row write: the same position's row, else the heading, else out of the vanished strip. */
	async function refocusRow(index: number) {
		await tick();
		const rows = [...document.querySelectorAll<HTMLElement>('section[aria-label="War room"] li')];
		const row = rows[Math.min(index, rows.length - 1)];
		if (row) row.querySelector<HTMLElement>('button')?.focus();
		else if (heading) heading.focus();
		else onempty?.();
	}

	function del(ref: ItemRef) {
		const params = { path: { key: ref.key } };
		if (ref.item === 'initiative') return unwrap(api.DELETE('/api/v1/initiatives/{key}/urgent', { params }));
		if (ref.item === 'epic') return unwrap(api.DELETE('/api/v1/epics/{key}/urgent', { params }));
		return unwrap(api.DELETE('/api/v1/stories/{key}/urgent', { params }));
	}
	function post(ref: ItemRef, reason: string | null) {
		const args = { params: { path: { key: ref.key } }, body: { reason } };
		if (ref.item === 'initiative') return unwrap(api.POST('/api/v1/initiatives/{key}/urgent', args));
		if (ref.item === 'epic') return unwrap(api.POST('/api/v1/epics/{key}/urgent', args));
		return unwrap(api.POST('/api/v1/stories/{key}/urgent', args));
	}

	async function ask(kind: 'clear' | 'renew') {
		confirm = { kind, items: (kind === 'clear' ? entries : renewables).map((e) => e.item) };
		await tick();
		document.querySelector<HTMLElement>('section[aria-label="War room"] [data-confirm]')?.focus();
	}

	async function cancel() {
		const kind = confirm?.kind;
		confirm = null;
		await tick();
		document.querySelector<HTMLElement>(`section[aria-label="War room"] [data-bulk="${kind}"]`)?.focus();
	}

	async function bulk() {
		const c = confirm;
		confirm = null;
		if (!c) return;
		const { ran, error } = await run(() =>
			unwrap(api.POST(c.kind === 'clear' ? '/api/v1/war-room/clear' : '/api/v1/war-room/renew', { body: { items: c.items } }))
		);
		if (!ran) return;
		if (error) writeError = error;
		await refocusRow(0);
	}

	async function clearRow(e: WarRoomEntry, index: number) {
		const { ran, error } = await run(() => del(e.item));
		if (!ran) return;
		if (error) writeError = error;
		await refocusRow(index);
	}

	async function remark(reason: string | null) {
		const target = remarking;
		if (!target) return;
		dialogError = null;
		const { ran, error } = await run(() => post(target.item, reason));
		if (!ran) return;
		if (error) dialogError = error;
		else {
			remarking = null;
			await refocusRow(0);
		}
	}

	/** "marked by dana, 2 d ago · 'reason' · expires in 5 d · done": the item pages' own-mark line, word for word. */
	const whyLine = (e: WarRoomEntry, now: Date) => [ownMarkText(e.mark, now), ...(e.mark.done ? ['done'] : [])].join(' · ');

	const route = (r: ItemRef) => itemRoute(r, page.url);
</script>

{#if entries.length}
	<section aria-label="War room" class="war-room">
		<div class="head">
			<h2 tabindex="-1" bind:this={heading}>
				<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"
					><path d="M5 2h2v20H5zM8 3h12l-3 5 3 5H8z" /></svg
				>War room
			</h2>
			{#if confirm}
				<span class="ask">
					{confirm.kind === 'clear' ? `Clear ${word(confirm.items.length)}?` : `Re-mark ${confirm.items.length} expired for another cycle?`}
					<button
						type="button"
						class="primary"
						data-confirm
						aria-label={confirm.kind === 'clear' ? `Clear ${word(confirm.items.length)}` : `Re-mark ${confirm.items.length} expired`}
						onclick={bulk}
						disabled={busy}>{confirm.kind === 'clear' ? 'Clear' : 'Re-mark'}</button
					>
					<button type="button" onclick={cancel}>Cancel</button>
				</span>
			{:else}
				<span class="bulk">
					{#if renewables.length}<button type="button" data-bulk="renew" onclick={() => ask('renew')} disabled={busy}
							>Re-mark all expired</button
						>{/if}
					<button type="button" data-bulk="clear" onclick={() => ask('clear')} disabled={busy}>End war room</button>
				</span>
			{/if}
		</div>
		{#if writeError}<p class="error" role="alert">{writeError}</p>{/if}
		<ul>
			{#each entries as e, i (e.item.item + e.item.key)}
				<li class:expired={!e.mark.active}>
					<span class="what">
						<a href={route(e.item)}>{e.title}</a>
						<span class="key">{e.item.item} {e.item.key}</span>
					</span>
					<span class="where">
						{#if e.initiative && e.item.item !== 'initiative'}in {e.initiative.title} · {e.initiative.horizon}
						{:else if e.initiative}{e.initiative.horizon}{/if}
					</span>
					<span class="why">{whyLine(e, at)}</span>
					<span class="row-actions">
						{#if renewable(e)}<button
								type="button"
								class="ghost"
								aria-label="Re-mark {e.item.key} {e.title}"
								onclick={() => ((dialogError = null), (remarking = e))}
								disabled={busy}>Re-mark</button
							>{/if}
						<button
							type="button"
							class="ghost"
							aria-label="Clear urgent mark on {e.item.key} {e.title}"
							onclick={() => clearRow(e, i)}
							disabled={busy}>Clear</button
						>
					</span>
				</li>
			{/each}
		</ul>
	</section>
{/if}

{#if remarking}
	<MarkUrgentDialog item={remarking.item.key} initial={remarking.mark.reason} {busy} error={dialogError} onsubmit={remark} oncancel={() => (remarking = null)} />
{/if}

<style>
	.war-room {
		margin-bottom: 16px;
		padding: 8px 12px;
		border-left: 3px solid var(--danger);
		background: var(--danger-soft);
		border-radius: 4px;
	}
	.head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 16px;
	}
	h2 {
		display: flex;
		align-items: center;
		gap: 6px;
		margin: 0;
		font-size: 15px;
		color: var(--danger);
	}
	.bulk,
	.ask {
		margin-left: auto;
		display: inline-flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px;
	}
	ul {
		display: grid;
		gap: 4px;
		margin: 8px 0 0;
		padding: 0;
		list-style: none;
	}
	li {
		display: flex;
		flex-wrap: wrap;
		align-items: baseline;
		gap: 2px 12px;
		font-size: 14px;
	}
	li.expired {
		color: var(--muted);
	}
	li.expired a {
		color: var(--muted);
	}
	.what a {
		font-weight: 600;
	}
	.key,
	.where,
	.why {
		color: var(--muted);
		font-size: 13px;
	}
	.row-actions {
		margin-left: auto;
		display: inline-flex;
		gap: 4px;
	}
	.error {
		margin: 4px 0 0;
		color: var(--danger);
	}
</style>
