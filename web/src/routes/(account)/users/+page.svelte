<script lang="ts">
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import type { Access, User } from '#lib/api/types.ts';
	import { allowedBy } from '#lib/access.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import { isAdmin, session } from '#lib/me.svelte.ts';

	let users = $state<User[] | null>(null);
	let access = $state<Access | null>(null);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);

	async function reload() {
		try {
			[users, access] = await Promise.all([unwrap(api.GET('/api/v1/users')), unwrap(api.GET('/api/v1/access'))]);
			loadError = null;
		} catch (e) {
			loadError = failure(e);
		}
	}

	// Only admins ask; the server refuses anyone else regardless. Loads once the viewer is
	// admin, including after a promotion while the page is open.
	let loaded = false;
	$effect(() => {
		if (isAdmin() && !loaded) {
			loaded = true;
			reload();
		}
	});

	const floorAdmin = (email: string) => access?.admins.some((r) => r.source === 'config' && r.value === email) ?? false;
	const dbAdmin = (email: string) => access?.admins.some((r) => r.source === 'database' && r.value === email) ?? false;

	function failure(e: unknown): string {
		if (e instanceof Refused) return e.reasons.map((r) => r.reason).join(', ');
		if (e instanceof Conflict) return e.message;
		if (e instanceof ApiError) return e.code === 'forbidden' ? 'Admins only.' : e.message;
		return errorMessage(e);
	}

	async function disable(u: User) {
		const ok = confirm(
			`Disable ${u.email}? This revokes every token and browser session they have, and they cannot sign in until enabled again.`
		);
		if (!ok) return;
		await act(() => unwrap(api.POST('/api/v1/users/{email}/disable', { params: { path: { email: u.email } } })));
	}

	async function enable(u: User) {
		if (!confirm(`Enable ${u.email}? They can sign in again; their old tokens stay revoked.`)) return;
		await act(() => unwrap(api.POST('/api/v1/users/{email}/enable', { params: { path: { email: u.email } } })));
	}

	async function act(f: () => Promise<unknown>) {
		busy = true;
		notice = null;
		try {
			await f();
			await reload();
		} catch (e) {
			notice = failure(e);
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head><title>Users · Reeve</title></svelte:head>

<PageHeader title="Users" />

{#if !isAdmin()}
	<p class="muted">Admins only.</p>
{:else}
	<p class="muted">Who may sign in and who is admin is managed on <a href="/access">Access</a>.</p>
	{#if notice}
		<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>
	{/if}
	{#if loadError}
		<Notice tone="error" onretry={reload}>Could not load users: {loadError}</Notice>
	{:else if users === null}
		<p class="muted">Loading…</p>
	{:else if users.length === 0}
		<p class="muted">No users yet.</p>
	{:else}
		<ul class="rows">
			{#each users as u (u.email)}
				<li class:disabled={u.disabled}>
					<span class="email" id="usr-{u.email}">{u.email}</span>
					<span class="name">{u.display_name || '—'}</span>
					<span class="muted">{u.role}</span>
					<span class="muted">{u.disabled ? 'disabled' : 'active'}</span>
					<span class="act">
						{#if u.email === session.me?.email}
							<span class="muted">you</span>
						{:else if floorAdmin(u.email)}
							<span class="muted">admin (reeve.toml)</span>
						{:else if access && allowedBy(access, u.email) === null}
							<!-- Make admin would silently re-allow them and the server refuses Enable; Disable still revokes their tokens. -->
							<span class="muted">not allowed (<a href="/access">manage on Access</a>)</span>
							{#if !u.disabled}
								<button type="button" disabled={busy} aria-describedby="usr-{u.email}" onclick={() => disable(u)}>Disable</button>
							{/if}
						{:else}
							{#if dbAdmin(u.email)}
								<button
									type="button"
									disabled={busy}
									aria-describedby="usr-{u.email}"
									onclick={() => act(() => unwrap(api.DELETE('/api/v1/access/admins/{email}', { params: { path: { email: u.email } } })))}
									>Revoke admin</button
								>
							{:else}
								<button
									type="button"
									disabled={busy}
									aria-describedby="usr-{u.email}"
									onclick={() => act(() => unwrap(api.POST('/api/v1/access/admins', { body: { email: u.email } })))}
									>Make admin</button
								>
							{/if}
							{#if u.disabled}
								<button type="button" disabled={busy} aria-describedby="usr-{u.email}" onclick={() => enable(u)}>Enable</button>
							{:else}
								<button type="button" disabled={busy} aria-describedby="usr-{u.email}" onclick={() => disable(u)}>Disable</button>
							{/if}
						{/if}
					</span>
				</li>
			{/each}
		</ul>
	{/if}
{/if}

<style>
	.email {
		flex: 1 1 14rem;
		overflow-wrap: anywhere;
	}
	.name {
		flex: 1 1 8rem;
	}
	.act {
		margin-left: auto;
	}
	.disabled {
		color: var(--muted);
	}
</style>
