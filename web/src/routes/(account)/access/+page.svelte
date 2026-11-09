<script lang="ts">
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import type { Access, AccessEntry, User } from '#lib/api/types.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { locksOut, norm, removalNote, type Removal } from '#lib/access.ts';
	import { tick } from 'svelte';
	import { errorMessage } from '#lib/format.ts';
	import { isAdmin, session } from '#lib/me.svelte.ts';
	import { onLive } from '#lib/events.svelte.ts';
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
	let asking = $state<
		({ what: 'entry'; removal: Removal } | { what: 'admin'; email: string }) & { opener: string } | null
	>(null);
	let dialogEl: HTMLElement | undefined = $state();

	// Opening the confirmation puts focus on its first button.
	$effect(() => {
		if (asking) tick().then(() => dialogEl?.querySelector('button')?.focus());
	});

	/** Back to the row's opener button, or the section's first input when the row is gone. Only if focus is free. */
	async function restore(opener: string, dialog: Element | null | undefined) {
		await tick();
		if (!focusIsFree(dialog)) return;
		const section = opener.split('|')[0];
		const btn = [...document.querySelectorAll<HTMLElement>('button[data-opener]')].find((b) => b.dataset.opener === opener);
		(btn ?? document.querySelector<HTMLElement>(`section[aria-label="${section}"] input`))?.focus();
	}
	async function focusForm(label: string, form: Element | null) {
		await tick();
		if (focusIsFree(form)) document.querySelector<HTMLElement>(`form[aria-label="${label}"] input`)?.focus();
	}
	function cancel() {
		if (!asking) return;
		const [opener, dlg] = [asking.opener, dialogEl];
		asking = null;
		restore(opener, dlg);
	}

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

	// Loads once the viewer is admin, including after a promotion while the page is open; a demotion
	// forgets the lists so a re-promotion loads fresh ones. Live changes reload, so a stale page
	// cannot act on an entry another admin has just removed.
	let loaded = false;
	$effect(() => {
		if (!isAdmin()) {
			loaded = false;
			access = null;
			users = [];
		} else if (!loaded) {
			loaded = true;
			reload();
		}
	});
	onLive(() => {
		if (isAdmin()) return reload();
	});

	/** Runs a write and shows the result. True when the write succeeded, even if the follow-up reload failed. */
	async function act(f: () => Promise<Access>, opener?: string): Promise<boolean> {
		busy = true;
		notice = null;
		const dlg = dialogEl;
		asking = null;
		let ok = false;
		try {
			access = await f();
			ok = true;
			users = await unwrap(api.GET('/api/v1/users'));
		} catch (e) {
			notice = ok ? `Saved, but the list of users could not be reloaded: ${failure(e)}` : failure(e);
		} finally {
			busy = false;
		}
		if (opener) restore(opener, dlg);
		return ok;
	}

	function submit(e: SubmitEvent, label: string, f: () => Promise<Access>, clear: () => void) {
		e.preventDefault();
		const form = e.currentTarget as Element;
		act(f).then((ok) => {
			if (ok) clear();
			focusForm(label, form);
		});
	}
	const allowGuest = (e: SubmitEvent) => {
		const value = guest;
		submit(
			e,
			'Allow a guest',
			() =>
				alsoAdmin
					? unwrap(api.POST('/api/v1/access/admins', { body: { email: value } }))
					: unwrap(api.POST('/api/v1/access/entries', { body: { kind: 'email', value } })),
			() => ([guest, alsoAdmin] = ['', false])
		);
	};
	const allowDomain = (e: SubmitEvent) => {
		const value = domain;
		submit(e, 'Allow a domain', () => unwrap(api.POST('/api/v1/access/entries', { body: { kind: 'domain', value } })), () => (domain = ''));
	};
	const grant = (e: SubmitEvent) => {
		const email = admin;
		submit(e, 'Grant admin', () => unwrap(api.POST('/api/v1/access/admins', { body: { email } })), () => (admin = ''));
	};
	const remove = (r: Removal, opener: string) =>
		act(() => unwrap(api.DELETE('/api/v1/access/entries/{kind}/{value}', { params: { path: { kind: r.kind, value: r.value } } })), opener);
	const revoke = (email: string, opener: string) =>
		act(() => unwrap(api.DELETE('/api/v1/access/admins/{email}', { params: { path: { email } } })), opener);
	const revokeNote = (email: string) =>
		access?.admins.find((r) => r.source === 'database' && r.value === email)?.also_in_config
			? `Revoke admin from ${email}? They stay admin through reeve.toml.`
			: `Revoke admin from ${email}? They keep member access while an entry allows them.`;

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
						{@const opener = `${kind === 'email' ? 'Guests' : 'Domains'}|${r.value}`}
						<button
							type="button"
							data-opener={opener}
							disabled={busy}
							onclick={() => (asking = { what: 'entry', removal: { kind, value: r.value }, opener })}>Remove</button
						>
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
		<div
			class="ask"
			role="alertdialog"
			tabindex="-1"
			aria-label={asking.what === 'entry' ? 'Confirm removal' : 'Confirm revoke'}
			aria-describedby="ask-text"
			bind:this={dialogEl}
			onkeydown={(e) => e.key === 'Escape' && cancel()}
		>
			{#if asking.what === 'entry'}
				{@const r = asking.removal}
				{@const op = asking.opener}
				<p id="ask-text">{removalNote(access!, users, r.kind, r.value)}</p>
				<button type="button" disabled={busy} onclick={() => remove(r, op)}>Yes, remove</button>
			{:else}
				{@const email = asking.email}
				{@const op = asking.opener}
				<p id="ask-text">{revokeNote(email)}</p>
				<button type="button" disabled={busy} onclick={() => revoke(email, op)}>Yes, revoke</button>
			{/if}
			<button type="button" class="ghost" onclick={cancel}>Cancel</button>
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
							{#if r.value !== norm(me)}
								{@const opener = `Admins|${r.value}`}
								<button
									type="button"
									data-opener={opener}
									disabled={busy}
									onclick={() => (asking = { what: 'admin', email: r.value, opener })}>Revoke</button
								>
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
