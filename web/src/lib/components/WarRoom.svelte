<!-- The war room strip: every urgent mark on one line each, active first. Loads its own data, so the planning page's
     "Mine" filter does not hide it. Clearing and renewing in bulk ask inline, never through a browser dialog. -->
<script lang="ts">
	import { api, ApiError, Refused, unwrap } from '#lib/api/client.ts';
	import type { ItemRef, WarRoomEntry } from '#lib/api/types.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { errorMessage, short } from '#lib/format.ts';
	import { expiry } from '#lib/urgency.ts';
	import MarkUrgentDialog from './MarkUrgentDialog.svelte';

	let { now = new Date() }: { now?: Date } = $props();

	let entries = $state<WarRoomEntry[]>([]);
	let confirming = $state<'clear' | 'renew' | null>(null);
	let remarking = $state<WarRoomEntry | null>(null);
	let dialogError = $state<string | null>(null);
	let error = $state<string | null>(null);
	let busy = $state(false);
	let latest = 0;

	const expired = $derived(entries.filter((e) => !e.mark.active));
	const word = (n: number) => `${n} ${n === 1 ? 'mark' : 'marks'}`;

	function show(e: unknown): string {
		if (e instanceof Refused) return `Refused: ${e.reasons.map((r) => r.reason.replaceAll('_', ' ')).join(', ')}`;
		if (e instanceof ApiError) return e.message;
		return errorMessage(e);
	}

	async function load() {
		const seq = ++latest;
		try {
			const list = await unwrap(api.GET('/api/v1/war-room'));
			if (seq === latest) {
				entries = list;
				error = null;
			}
		} catch (e) {
			if (seq === latest) error = `Could not load the war room: ${show(e)}`;
		}
	}
	void load();
	onLive(load);

	/** A write, then the refreshed list; a failure shows in the strip. */
	async function run(write: () => Promise<unknown>): Promise<boolean> {
		if (busy) return false;
		busy = true;
		error = null;
		try {
			await write();
			await load();
			return true;
		} catch (e) {
			error = show(e);
			return false;
		} finally {
			busy = false;
		}
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

	async function bulk() {
		const kind = confirming;
		confirming = null;
		if (!kind) return;
		const items = (kind === 'clear' ? entries : expired).map((e) => e.item);
		await run(() => unwrap(api.POST(kind === 'clear' ? '/api/v1/war-room/clear' : '/api/v1/war-room/renew', { body: { items } })));
	}

	async function remark(reason: string | null) {
		const target = remarking;
		if (!target) return;
		dialogError = null;
		if (await run(() => post(target.item, reason))) remarking = null;
		else {
			dialogError = error;
			error = null;
		}
	}
</script>

{#if entries.length || error}
	<section aria-label="War room" class="war-room" class:quiet={!entries.length}>
		<div class="head">
			<h2>
				<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"
					><path d="M5 2h2v20H5zM8 3h12l-3 5 3 5H8z" /></svg
				>War room
			</h2>
			{#if confirming}
				<span class="ask">
					{confirming === 'clear' ? `Clear ${word(entries.length)}?` : `Re-mark ${expired.length} expired?`}
					<button type="button" class="primary" onclick={bulk} disabled={busy}>{confirming === 'clear' ? 'Clear' : 'Re-mark'}</button>
					<button type="button" onclick={() => (confirming = null)}>Cancel</button>
				</span>
			{:else if entries.length}
				<span class="bulk">
					{#if expired.length}<button type="button" onclick={() => (confirming = 'renew')} disabled={busy}>Re-mark all expired</button>{/if}
					<button type="button" onclick={() => (confirming = 'clear')} disabled={busy}>End war room</button>
				</span>
			{/if}
		</div>
		{#if error}<p class="error" role="alert">{error}</p>{/if}
		<ul>
			{#each entries as e (e.item.item + e.item.key)}
				<li class:expired={!e.mark.active}>
					<span class="what">
						<a href="/{e.item.item === 'initiative' ? 'initiatives' : e.item.item === 'epic' ? 'epics' : 'stories'}/{encodeURIComponent(e.item.key)}"
							>{e.title}</a
						>
						<span class="key">{e.item.key}</span>
					</span>
					<span class="where">
						{#if e.initiative && e.item.item !== 'initiative'}in {e.initiative.title} · {e.initiative.horizon}
						{:else if e.initiative}{e.initiative.horizon}{/if}
					</span>
					<span class="why">
						{#if e.mark.reason}{e.mark.reason} · {/if}marked by {short(e.mark.by)} ·
						{expiry(e.mark, now)}{#if e.mark.done}
							· done{/if}
					</span>
					<span class="row-actions">
						{#if !e.mark.active}<button type="button" class="ghost" onclick={() => ((dialogError = null), (remarking = e))} disabled={busy}>Re-mark</button>{/if}
						<button type="button" class="ghost" onclick={() => run(() => del(e.item))} disabled={busy}>Clear</button>
					</span>
				</li>
			{/each}
		</ul>
	</section>
{/if}

{#if remarking}
	<MarkUrgentDialog
		initial={remarking.mark.reason}
		{busy}
		error={dialogError}
		onsubmit={remark}
		oncancel={() => (remarking = null)}
	/>
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
