<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import type { Token } from '#lib/api/types.ts';
	import ConnectedApps from '#lib/components/ConnectedApps.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';
	import { isInteractive } from '#lib/me.svelte.ts';
	import { liveBrowserSession } from '#lib/tokens.ts';

	type Kind = Token['kind'];

	let tokens = $state<Token[] | null>(null);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);

	// `?for=cli` arrives from `reeve auth login`: an interactive token labelled "cli", pasted back into the terminal.
	const forCli = $derived(page.url.searchParams.get('for') === 'cli');
	let kind = $state<Kind>('interactive');
	let label = $state('');
	let days = $state<number | null>(null);
	$effect(() => {
		if (forCli) {
			kind = 'interactive';
			label = 'cli';
		}
	});

	/** The one-time secret of the token just issued; it lives only in this component and is never fetched again. */
	let issued = $state<{ secret: string; kind: Kind; label: string } | null>(null);
	let copied = $state(false);

	// The URL people use to reach this instance; the CLI needs the same one.
	const origin = $derived(page.url.origin);

	async function reload() {
		try {
			tokens = await unwrap(api.GET('/api/v1/tokens'));
			loadError = null;
		} catch (e) {
			loadError = failure(e);
		}
	}

	onMount(reload);

	function failure(e: unknown): string {
		if (e instanceof Refused) return e.reasons.map((r) => r.reason).join(', ');
		if (e instanceof Conflict) return e.message;
		if (e instanceof ApiError) return e.code === 'forbidden' ? 'Automation tokens cannot issue tokens.' : e.message;
		return errorMessage(e);
	}

	async function create(ev: SubmitEvent) {
		ev.preventDefault();
		if (busy) return;
		busy = true;
		notice = null;
		issued = null;
		copied = false;
		try {
			const out = await unwrap(
				api.POST('/api/v1/tokens', {
					body: { kind, label: label.trim(), expires_in_days: days || null }
				})
			);
			issued = { secret: out.secret, kind: out.token.kind, label: out.token.label };
			label = '';
			days = null;
			await reload();
		} catch (e) {
			notice = failure(e);
		} finally {
			busy = false;
		}
	}

	async function copy() {
		if (!issued) return;
		try {
			await navigator.clipboard.writeText(issued.secret);
			copied = true;
		} catch {
			notice = 'Could not copy; select the token and copy it by hand.';
		}
	}

	async function revoke(t: Token) {
		const what = t.label ? `"${t.label}"` : `#${t.id}`;
		if (!confirm(`Revoke the ${t.kind} token ${what}? Anything using it stops working.`)) return;
		busy = true;
		notice = null;
		try {
			await unwrap(api.DELETE('/api/v1/tokens/{id}', { params: { path: { id: t.id } } }));
			await reload();
		} catch (e) {
			notice = failure(e);
		} finally {
			busy = false;
		}
	}


	function day(iso: string | null | undefined): string {
		return iso ? iso.slice(0, 10) : '—';
	}
</script>

<svelte:head><title>Tokens · Reeve</title></svelte:head>

<PageHeader title="Tokens" />

{#if notice}
	<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>
{/if}

{#if issued}
	<section class="issued" aria-labelledby="issued-title">
		<h2 id="issued-title">New {issued.kind} token{issued.label ? ` "${issued.label}"` : ''}</h2>
		<p><strong>Copy it now; it is not shown again.</strong></p>
		<div class="copybox">
			<input readonly value={issued.secret} aria-label="token" onfocus={(e) => e.currentTarget.select()} />
			<button type="button" onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
		</div>
		{#if forCli && issued.kind === 'interactive'}
			<p>Paste the token into the waiting <code>reeve auth login</code>.</p>
		{/if}
		<p>For the CLI on your machine (an interactive token), paste it when asked:</p>
		<pre><code>reeve auth set-token --url {origin} --token-stdin</code></pre>
		<p>For automation (an automation token), pass it in the environment:</p>
		<pre><code>REEVE_URL={origin} REEVE_TOKEN=… reeve …</code></pre>
	</section>
{/if}

{#if isInteractive()}
	<form class="new" onsubmit={create}>
		<h2>New token</h2>
		{#if forCli}
			<p class="hint">Create this token, then paste it into the waiting <code>reeve auth login</code>.</p>
		{/if}
		<label>
			Kind
			<select bind:value={kind}>
				<option value="interactive">interactive (you, at the CLI)</option>
				<option value="automation">automation (scripts and agents)</option>
			</select>
		</label>
		<label>
			Label
			<input bind:value={label} maxlength="100" placeholder="laptop, ci, …" />
		</label>
		<label>
			Expires in days
			<input type="number" min="1" max="365" bind:value={days} placeholder="never" />
		</label>
		<button type="submit" disabled={busy}>Create token</button>
	</form>
{:else}
	<p class="muted">This session uses an automation token; only an interactive session can issue tokens.</p>
{/if}

<h2>Your tokens</h2>
{#if loadError}
	<Notice tone="error" onretry={reload}>Could not load tokens: {loadError}</Notice>
{:else if tokens === null}
	<p class="muted">Loading…</p>
{:else if tokens.length === 0}
	<p class="muted">No tokens yet.</p>
{:else}
	<ul class="rows">
		{#each tokens as t (t.id)}
			<li class:revoked={!!t.revoked_at}>
				<span class="kind">{t.kind}</span>
				<span class="lbl" id="tok-{t.id}">
					{t.label || '—'}
					{#if liveBrowserSession(t, new Date())}<span class="chip" title="Issued by signing in from a browser; one of them is this one">browser session</span>{/if}
				</span>
				<span class="muted" title={t.created_at}>Created {day(t.created_at)}</span>
				<span class="muted" title={t.expires_at ?? ''}>Expires {t.expires_at ? day(t.expires_at) : 'never'}</span>
				{#if t.revoked_at}<span class="muted" title={t.revoked_at}>Revoked {day(t.revoked_at)}</span>{/if}
				{#if !t.revoked_at}
					<button type="button" class="act" disabled={busy} aria-describedby="tok-{t.id}" onclick={() => revoke(t)}>Revoke</button>
				{/if}
			</li>
		{/each}
	</ul>
{/if}

<h2>Connected apps</h2>
<p class="muted">Apps you connected to Reeve from a chat client. Each ends after 30 days; revoke one to cut it now.</p>
<ConnectedApps />

<style>
	h2 {
		margin: 20px 0 8px;
	}
	.issued {
		max-width: 48rem;
		padding: 0.75rem 1rem;
		border: 1px solid var(--accent);
		border-radius: 8px;
	}
	.issued h2 {
		margin-top: 0;
	}
	.copybox {
		display: flex;
		gap: 0.5rem;
	}
	.copybox input {
		flex: 1;
		font-family: var(--mono);
	}
	pre {
		overflow-x: auto;
		padding: 0.5rem;
		border-radius: 4px;
		background: var(--lighter-background);
	}
	.new {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem 1rem;
		align-items: end;
		max-width: 48rem;
		margin: 1rem 0;
	}
	.new h2,
	.new .hint {
		flex-basis: 100%;
		margin: 0;
	}
	.new label {
		display: grid;
		gap: 0.2rem;
	}
	.kind {
		min-width: 6rem;
		font-weight: 600;
	}
	.lbl {
		flex: 1 1 10rem;
	}
	.act {
		margin-left: auto;
	}
	.chip {
		margin-left: 0.4rem;
		padding: 0 0.4rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		color: var(--muted);
		font-size: 13px;
	}
	.revoked {
		color: var(--muted);
	}
</style>
