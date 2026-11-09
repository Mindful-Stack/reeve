<!-- web/src/routes/(account)/+layout.svelte: the account pages share a side menu. -->
<script lang="ts">
	import { page } from '$app/state';
	import { isAdmin, session } from '#lib/me.svelte.ts';
	import type { Snippet } from 'svelte';

	let { children }: { children: Snippet } = $props();
	const items = $derived([
		{ href: '/tokens', label: 'Tokens' },
		{ href: '/activity', label: 'Activity' },
		...(isAdmin() ? [{ href: '/users', label: 'Users' }, { href: '/access', label: 'Access' }] : [])
	]);
</script>

<div class="account">
	<nav aria-label="Account">
		<p class="muted who">{session.me?.email}</p>
		{#each items as i (i.href)}
			<a href={i.href} aria-current={page.url.pathname === i.href ? 'page' : undefined}>{i.label}</a>
		{/each}
	</nav>
	<div class="content">{@render children()}</div>
</div>

<style>
	.account {
		display: flex;
		flex-wrap: wrap;
		gap: 24px;
		max-width: 1000px;
		margin: 0 auto;
	}
	nav {
		display: grid;
		align-content: start;
		gap: 2px;
		flex: 1 1 180px;
		max-width: 220px;
	}
	.who {
		margin: 0 0 8px;
		padding: 0 10px;
		font-size: 14px;
		overflow-wrap: anywhere;
	}
	nav a {
		padding: 6px 10px;
		border-radius: 4px;
		color: var(--muted);
		text-decoration: none;
	}
	nav a[aria-current='page'] {
		background: var(--lighter-background);
		color: var(--foreground);
		font-weight: 600;
	}
	.content {
		flex: 999 1 560px;
		min-width: 0;
	}
	@media (max-width: 720px) {
		nav {
			display: flex;
			flex-wrap: wrap;
			max-width: none;
		}
		.who {
			flex-basis: 100%;
		}
	}
</style>
