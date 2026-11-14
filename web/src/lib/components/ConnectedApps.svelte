<!-- web/src/lib/components/ConnectedApps.svelte: the apps a person connected through MCP sign-in (kb/ADR-0029 R9).
     Without `owner` it lists the viewer's own; with one (the users page, admins) that person's. -->
<script lang="ts">
	import { onMount } from 'svelte';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { Connection } from '#lib/api/types.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { errorMessage } from '#lib/format.ts';
	import Notice from '#lib/ui/Notice.svelte';

	let { owner }: { owner?: string } = $props();

	let rows = $state<Connection[] | null>(null);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);

	async function reload() {
		try {
			rows = await unwrap(api.GET('/api/v1/connections', { params: { query: owner ? { owner } : {} } }));
			loadError = null;
		} catch (e) {
			loadError = errorMessage(e);
		}
	}
	onMount(reload);
	// A revoke elsewhere or a disable is an activity entry, so the list follows the live stream. A refresh writes
	// none: "Last refreshed" is as fresh as the last load.
	onLive(reload);

	const day = (iso: string) => iso.slice(0, 10);
	const app = (c: Connection) => c.client_host || c.client_name;

	/** Why a connection no longer works, or null while it does. */
	function stopped(c: Connection, now: number): string | null {
		if (c.revoked_at) {
			const on = day(c.revoked_at);
			switch (c.revoked_reason) {
				case 'refresh_token_reused':
					return `Ended ${on}: the app’s sign-in went out of step; connect it again`;
				case 'user_disabled':
					return `Ended ${on}: the account was disabled`;
				case 'restored':
					return `Ended ${on}: the instance was restored from a backup`;
				default:
					return `Revoked ${on}`;
			}
		}
		return Date.parse(c.ends_at) <= now ? `Ended ${day(c.ends_at)}` : null;
	}

	async function revoke(c: Connection) {
		const whose = owner ? ` for ${owner}` : '';
		if (!confirm(`Revoke ${app(c)}${whose}? It stops working at once and has to be connected again.`)) return;
		busy = true;
		notice = null;
		try {
			await unwrap(api.DELETE('/api/v1/connections/{id}', { params: { path: { id: c.id } } }));
			await reload();
		} catch (e) {
			notice = errorMessage(e);
		} finally {
			busy = false;
		}
	}
</script>

{#if notice}
	<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>
{/if}
{#if loadError}
	<Notice tone="error" onretry={reload}>Could not load connected apps: {loadError}</Notice>
{:else if rows === null}
	<p class="muted">Loading…</p>
{:else if rows.length === 0}
	<p class="muted">No connected apps.</p>
{:else}
	<ul class="rows">
		{#each rows as c (c.id)}
			{@const why = stopped(c, Date.now())}
			<li data-connection={c.id} class:stopped={why !== null}>
				<span class="app" id="con-{c.id}">{app(c)}</span>
				{#if c.client_host}<span class="muted">{c.client_name}</span>{/if}
				<span>{c.scope === 'write' ? 'Read and change' : 'Read'}</span>
				<span class="muted" title={c.approved_at}>Connected {day(c.approved_at)}</span>
				<span class="muted" title={c.revoked_at ?? c.ends_at}>{why ?? `Ends ${day(c.ends_at)}`}</span>
				<span class="muted" title={c.refreshed_at ?? ''}>Last refreshed {c.refreshed_at ? day(c.refreshed_at) : 'never'}</span>
				{#if why === null}
					<button type="button" class="act" disabled={busy} aria-describedby="con-{c.id}" onclick={() => revoke(c)}>Revoke</button>
				{/if}
			</li>
		{/each}
	</ul>
{/if}

<style>
	.app {
		flex: 1 1 10rem;
		font-weight: 600;
		overflow-wrap: anywhere;
	}
	.act {
		margin-left: auto;
	}
	.stopped {
		color: var(--muted);
	}
</style>
