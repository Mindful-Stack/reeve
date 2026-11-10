<script lang="ts">
	import { page } from '$app/state';
	import { onMount } from 'svelte';
	import Logo from '#lib/components/Logo.svelte';

	// Plain fetch, never `unwrap`: this page renders signed out, and /auth/status is not an API envelope.
	let signIn = $state<boolean | null>(null);
	let failed = $state(false);

	// The server re-filters `next` to a same-origin path; the client only passes it through.
	const next = $derived(page.url.searchParams.get('next') ?? '/');
	const notAllowed = $derived(page.url.searchParams.get('reason') === 'not_allowed');

	const loginHref = $derived(`/auth/login?next=${encodeURIComponent(next)}`);

	onMount(async () => {
		try {
			const res = await fetch('/auth/status', { credentials: 'omit' });
			if (!res.ok) throw new Error(res.statusText);
			signIn = ((await res.json()) as { sign_in: boolean }).sign_in;
		} catch {
			failed = true;
		}
	});
</script>

<svelte:head><title>Sign in · Reeve</title></svelte:head>

<main class="signin-page">
	<div class="card">
		<h1 class="title"><Logo />Reeve</h1>
		<p class="muted">Plan the work, then build it.</p>
		{#if notAllowed}
			<p role="alert">This address is not allowed on this instance.</p>
		{/if}
		{#if failed}
			<p role="alert">Could not reach the server.</p>
		{:else if signIn === true}
			<a class="button primary" href={loginHref}>Sign in with Google</a>
			<p class="muted small">Only invited accounts can sign in.</p>
		{:else if signIn === false}
			<p>
				Sign-in is not configured on this instance. An operator can issue a token with
				<code>reeve-server token-issue --email you@your-domain --kind interactive</code> for the CLI.
			</p>
		{/if}
	</div>
</main>

<style>
	.signin-page {
		min-height: 100vh;
		display: grid;
		place-items: center;
		padding: 0; /* overrides main's page padding */
	}
	.card {
		width: min(360px, 100% - 32px);
		box-sizing: border-box;
		padding: 28px 24px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
		display: grid;
		gap: 16px;
		text-align: center;
	}
	.card > * {
		margin: 0;
	}
	.title {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 8px;
	}
</style>
