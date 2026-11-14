<!-- web/src/routes/connect/+page.svelte: the approval page of MCP sign-in (kb/ADR-0029 R4). A signed-out page: it
     calls the API itself, and `unwrap` sends a person without a session to sign in and back here. -->
<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { api, ApiError, unwrap } from '#lib/api/client.ts';
	import type { ConnectionRequest, Scope } from '#lib/api/types.ts';
	import { safeDestination } from '#lib/connect.ts';
	import { age } from '#lib/format.ts';
	import Logo from '#lib/components/Logo.svelte';

	// The instance is this page's own host, which is public_url's host.
	const instance = $derived(page.url.host);
	const id = $derived(page.url.searchParams.get('request'));
	// Authorize sends a problem instead of a request when nothing can be approved; an address with neither is one too.
	const problem = $derived(page.url.searchParams.get('problem') || (id ? null : 'invalid_request'));

	let view = $state<'loading' | 'request' | 'expired' | 'unreachable' | 'invalid'>('loading');
	let request = $state<ConnectionRequest | null>(null);
	// Neither is preselected: an app asks for every scope the server lists, so the choice is the person's.
	let scope = $state<Scope | null>(null);
	let busy = $state(false);
	let failed = $state(false);

	const notFound = (e: unknown) => e instanceof ApiError && e.code === 'not_found';

	async function load() {
		if (!id) return;
		view = 'loading';
		try {
			request = await unwrap(api.GET('/api/v1/connection-requests/{id}', { params: { path: { id } } }));
			view = 'request';
		} catch (e) {
			view = notFound(e) ? 'expired' : 'unreachable';
		}
	}
	onMount(() => {
		if (!problem) void load();
	});

	async function answer(approve: boolean) {
		if (!id || !request || busy || (approve && scope === null)) return;
		busy = true;
		failed = false;
		const path = { id };
		const approval = request.approval;
		try {
			const out = approve
				? await unwrap(api.POST('/api/v1/connection-requests/{id}/approve', { params: { path }, body: { approval, scope: scope! } }))
				: await unwrap(api.POST('/api/v1/connection-requests/{id}/refuse', { params: { path }, body: { approval } }));
			if (!safeDestination(out.redirect_to)) {
				view = 'invalid';
				return;
			}
			// A full navigation to the app. `busy` stays set: the request is answered, a second click could only fail.
			location.assign(out.redirect_to);
		} catch (e) {
			busy = false;
			if (notFound(e)) view = 'expired';
			else failed = true;
		}
	}

	async function signOut() {
		await fetch('/auth/logout', { method: 'POST', headers: { 'X-Reeve-Client': 'web' } });
		location.href = '/auth/signin';
	}
</script>

<svelte:head><title>Connect an app · Reeve</title></svelte:head>

<main class="connect-page">
	<div class="card">
		<p class="title"><Logo />Reeve</p>
		{#if problem === 'app_not_enabled'}
			<p role="alert">This app is not enabled on {instance}. Ask an admin.</p>
		{:else if problem || view === 'invalid'}
			<p role="alert">This connection request was not valid. Nothing was shared.</p>
		{:else if view === 'loading'}
			<p class="muted" role="status">Loading…</p>
		{:else if view === 'expired'}
			<p role="alert">This connection request has expired or was opened in another browser. Go back to the app and connect again.</p>
		{:else if view === 'unreachable'}
			<p role="alert">Could not reach {instance}.</p>
			<button type="button" onclick={load}>Try again</button>
		{:else if request}
			<h1>{request.client_host} wants to connect to {instance}</h1>
			<p class="muted small">The app calls itself “{request.client_name}”.</p>
			<p>
				Signed in as {request.email}. Wrong account?
				<button type="button" class="link" onclick={signOut}>Sign out</button>, then connect again from the app.
			</p>
			<fieldset>
				<legend id="scope-legend">What may it do?</legend>
				<label><input type="radio" name="scope" value="read" bind:group={scope} /> Read: everything you can see in {instance}</label>
				<label><input type="radio" name="scope" value="write" bind:group={scope} /> Read and change: also create, edit, comment on and move stories as you</label>
			</fieldset>
			<ul>
				<li>What the app reads is sent to its provider.</li>
				<li>It can never override the readiness gate, issue tokens or change who has access.</li>
				<li>The connection ends after 30 days, and you can revoke it at any time.</li>
			</ul>
			<p class="muted small">Requested {age(request.requested_at)} ago. You will be sent back to {request.redirect_host}.</p>
			<p class="warning">
				<strong>If someone sent you this link, refuse.</strong> Approving gives their {request.client_host} account access to {instance} as you.
			</p>
			{#if failed}
				<p role="alert">Could not reach {instance}. Try again.</p>
			{/if}
			<div class="actions">
				<!-- Disabled until a scope is chosen: the legend says what it waits for. -->
				<button type="button" class="primary" aria-describedby="scope-legend" disabled={busy || scope === null} onclick={() => answer(true)}>Approve</button>
				<button type="button" disabled={busy} onclick={() => answer(false)}>Refuse</button>
			</div>
		{/if}
	</div>
</main>

<style>
	.connect-page {
		min-height: 100vh;
		display: grid;
		place-items: center;
		padding: 0; /* overrides main's page padding */
	}
	.card {
		width: min(480px, 100% - 32px);
		box-sizing: border-box;
		padding: 28px 24px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
		display: grid;
		gap: 14px;
	}
	.card > * {
		margin: 0;
	}
	.title {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 18px;
		font-weight: 700;
	}
	h1 {
		font-size: 20px;
		overflow-wrap: anywhere;
	}
	fieldset {
		display: grid;
		gap: 8px;
		padding: 10px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
	}
	ul {
		padding-left: 20px;
	}
	.warning {
		padding: 8px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--lighter-background);
	}
	.actions {
		display: flex;
		gap: 12px;
	}
	.link {
		padding: 0;
		border: 0;
		background: none;
		color: var(--accent);
		text-decoration: underline;
		cursor: pointer;
	}
</style>
