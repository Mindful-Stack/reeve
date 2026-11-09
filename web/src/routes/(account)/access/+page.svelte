<script lang="ts">
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import type { Access, AccessEntry, User } from '#lib/api/types.ts';
	import { locksOut, removalNote, type Removal } from '#lib/access.ts';
	import { errorMessage } from '#lib/format.ts';
	import { isAdmin, session } from '#lib/me.svelte.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';

	let access = $state<Access | null>(null);
	let users = $state<User[]>([]);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);
	let guest = $state('');
	let alsoAdmin = $state(false);
	let domain = $state('');
	let admin = $state('');
	/** The removal awaiting its inline confirmation. */
	let asking = $state<{ what: 'entry'; removal: Removal } | { what: 'admin'; email: string } | null>(null);

	function failure(e: unknown): string {
		if (e instanceof Refused) return e.reasons.map((r) => r.reason).join(', ');
		if (e instanceof Conflict) return e.message;
		if (e instanceof ApiError) return e.code === 'forbidden' ? 'Admins only.' : e.message;
		return errorMessage(e);
	}

	async function reload() {
		try {
			[access, users] = await Promise.all([unwrap(api.GET('/api/v1/access')), unwrap(api.GET('/api/v1/users'))]);
			loadError = null;
		} catch (e) {
			loadError = failure(e);
		}
	}

	// Loads once the viewer is admin, including after a promotion while the page is open.
	let loaded = false;
	$effect(() => {
		if (isAdmin() && !loaded) {
			loaded = true;
			reload();
		}
	});

	async function act(f: () => Promise<Access>) {
		busy = true;
		notice = null;
		asking = null;
		try {
			access = await f();
			users = await unwrap(api.GET('/api/v1/users'));
		} catch (e) {
			notice = failure(e);
		} finally {
			busy = false;
		}
	}

	const allowGuest = (e: SubmitEvent) => {
		e.preventDefault();
		const value = guest;
		act(() =>
			alsoAdmin
				? unwrap(api.POST('/api/v1/access/admins', { body: { email: value } }))
				: unwrap(api.POST('/api/v1/access/entries', { body: { kind: 'email', value } }))
		).then(() => {
			if (!notice) [guest, alsoAdmin] = ['', false];
		});
	};
	const allowDomain = (e: SubmitEvent) => {
		e.preventDefault();
		const value = domain;
		act(() => unwrap(api.POST('/api/v1/access/entries', { body: { kind: 'domain', value } }))).then(() => {
			if (!notice) domain = '';
		});
	};
	const grant = (e: SubmitEvent) => {
		e.preventDefault();
		const email = admin;
		act(() => unwrap(api.POST('/api/v1/access/admins', { body: { email } }))).then(() => {
			if (!notice) admin = '';
		});
	};
	const remove = (r: Removal) =>
		act(() => unwrap(api.DELETE('/api/v1/access/entries/{kind}/{value}', { params: { path: { kind: r.kind, value: r.value } } })));
	const revoke = (email: string) => act(() => unwrap(api.DELETE('/api/v1/access/admins/{email}', { params: { path: { email } } })));

	const me = $derived(session.me?.email ?? '');
	const added = (r: AccessEntry) => `added by ${r.added_by}, ${r.added_at?.slice(0, 10)}`;
</script>

<svelte:head><title>Access · Reeve</title></svelte:head>

<PageHeader title="Access" />

{#snippet entries(kind: 'domain' | 'email', rows: AccessEntry[], prefix: string)}
	<ul class="rows">
		{#each rows as r (r.source + r.value)}
			<li>
				<span class="value">{prefix}{r.value}</span>
				{#if r.source === 'config'}
					<span class="muted">from reeve.toml</span>
				{:else}
					<span class="muted">{added(r)}{r.also_in_config ? ' · also in reeve.toml' : ''}</span>
					{#if !locksOut(access!, me, kind, r.value)}
						<button type="button" disabled={busy} onclick={() => (asking = { what: 'entry', removal: { kind, value: r.value } })}>Remove</button>
					{/if}
				{/if}
			</li>
		{/each}
	</ul>
{/snippet}

{#if !isAdmin()}
	<p class="muted">Admins only.</p>
{:else}
	<p class="muted">
		Who may sign in and who is admin. Entries from <code>reeve.toml</code> are locked here; change them there.
	</p>
	{#if notice}
		<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>
	{/if}
	{#if asking}
		<div class="ask" role="alertdialog" aria-label="Confirm">
			{#if asking.what === 'entry'}
				{@const r = asking.removal}
				<p>{removalNote(access!, users, r.kind, r.value)}</p>
				<button type="button" disabled={busy} onclick={() => remove(r)}>Yes, remove</button>
			{:else}
				{@const email = asking.email}
				<p>Revoke admin from {email}? They keep member access while an entry allows them.</p>
				<button type="button" disabled={busy} onclick={() => revoke(email)}>Yes, revoke</button>
			{/if}
			<button type="button" class="ghost" onclick={() => (asking = null)}>Cancel</button>
		</div>
	{/if}
	{#if loadError}
		<Notice tone="error" onretry={reload}>Could not load access: {loadError}</Notice>
	{:else if access === null}
		<p class="muted">Loading…</p>
	{:else}
		<section aria-label="Guests">
			<h2>Guests (emails)</h2>
			<form aria-label="Allow a guest" onsubmit={allowGuest}>
				<input type="email" required placeholder="guest@partner.com" bind:value={guest} aria-label="Guest email" />
				<label><input type="checkbox" bind:checked={alsoAdmin} /> also admin</label>
				<button disabled={busy}>Allow</button>
			</form>
			{@render entries('email', access.emails, '')}
		</section>
		<section aria-label="Domains">
			<h2>Domains</h2>
			<form aria-label="Allow a domain" onsubmit={allowDomain}>
				<input required placeholder="partner.com" bind:value={domain} aria-label="Domain" />
				<button disabled={busy}>Allow</button>
			</form>
			{@render entries('domain', access.domains, '@')}
		</section>
		<section aria-label="Admins">
			<h2>Admins</h2>
			<form aria-label="Grant admin" onsubmit={grant}>
				<input type="email" required placeholder="someone@example.com" bind:value={admin} aria-label="Admin email" />
				<button disabled={busy}>Grant admin</button>
			</form>
			<ul class="rows">
				{#each access.admins as r (r.source + r.value)}
					<li class:inert={!r.allowed}>
						<span class="value">{r.value}</span>
						{#if r.source === 'config'}
							<span class="muted">from reeve.toml</span>
						{:else}
							<span class="muted">{added(r)}{r.also_in_config ? ' · also in reeve.toml' : ''}</span>
							{#if !r.allowed}<span class="muted">not allowed: grants nothing until their email is allowed again</span>{/if}
							{#if r.value !== me}
								<button type="button" disabled={busy} onclick={() => (asking = { what: 'admin', email: r.value })}>Revoke</button>
							{/if}
						{/if}
					</li>
				{/each}
			</ul>
		</section>
	{/if}
{/if}

<style>
	section {
		margin-top: 24px;
	}
	form {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		margin-bottom: 8px;
	}
	.value {
		flex: 1 1 14rem;
		overflow-wrap: anywhere;
	}
	.inert {
		color: var(--muted);
	}
	.ask {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		margin: 12px 0;
	}
	.ask p {
		flex-basis: 100%;
		margin: 0;
	}
</style>
