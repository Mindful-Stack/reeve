<script lang="ts">
	import '../app.css';
	import '@fontsource-variable/instrument-sans/index.css';
	import '@fontsource/jetbrains-mono/400.css';
	import '@fontsource/jetbrains-mono/500.css';
	import { tick, untrack } from 'svelte';
	import { goto, onNavigate } from '$app/navigation';
	import { page } from '$app/state';
	import { api, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import { connect, live } from '#lib/events.svelte.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { followAccess, isAdmin, load, session } from '#lib/me.svelte.ts';
	import { headerSearch } from '#lib/headerSearch.ts';
	import { isCurrent, links, reportContext } from '#lib/nav.ts';
	import { discardDraft, hasDraft, openReport, persist, report, reportLabel, restoreDraft, type EpicRef } from '#lib/report.svelte.ts';
	import type { Story } from '#lib/api/types.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import ReportDialog from '#lib/components/ReportDialog.svelte';
	import ActionsMenu from '#lib/ui/ActionsMenu.svelte';
	import Avatar from '#lib/ui/Avatar.svelte';
	import StoryDrawer from '#lib/components/StoryDrawer.svelte';
	import Logo from '#lib/components/Logo.svelte';
	import GoalPicker from '#lib/components/GoalPicker.svelte';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();

	// /auth/* renders signed out: loading the session there would 401 and redirect to next=/auth/signin forever.
	const signedOutPage = $derived(page.url.pathname.startsWith('/auth/'));

	const accountItems = $derived([
		{ label: 'Tokens', onselect: () => goto('/tokens') },
		{ label: 'Activity', onselect: () => goto('/activity') },
		...(isAdmin() ? [{ label: 'Users', onselect: () => goto('/users') }, { label: 'Access', onselect: () => goto('/access') }] : []),
		{ label: 'Sign out', onselect: signOut }
	]);
	let search: HTMLInputElement | undefined = $state();
	let q = $state('');
	// On /backlog and /search the field shows the URL's text, trimmed as those pages read it, so a reload keeps it. Only
	// when that text changes: toggling a filter or opening the drawer never rewrites what is being typed.
	/** The text the box mirrors on `url`: the URL's `q` on /backlog and /search, null elsewhere. */
	const mirrorOf = (url: { readonly pathname: string; readonly searchParams: Pick<URLSearchParams, 'get'> }) => (url.pathname === '/backlog' || url.pathname === '/search' ? (url.searchParams.get('q')?.trim() ?? '') : null);
	let mirrored: string | null = null;
	$effect(() => {
		const text = mirrorOf(page.url);
		if (text !== null && text !== mirrored) q = text;
		mirrored = text;
	});

	// The one text box (kb/ADR-0021): a key jumps to the item it names, any other text filters the backlog
	// (#lib/headerSearch.ts). A newer submission supersedes a pending lookup, and a lookup whose page moved on is dropped.
	let searching = $state(false);
	const headerBox = headerSearch(
		(text) => unwrap(api.GET('/api/v1/search', { params: { query: { q: text } } })),
		async (to) => {
			// Text on the backlog edits its URL in place, as its filters do.
			if (to.kind === 'filter') return page.url.pathname === '/backlog' ? goto(to.href, { replace: true, reset: false }) : goto(to.href);
			// A story opens over the current view. After a jump the key is spent: the box shows the page's own text again.
			await (to.kind === 'story' ? goto(to.href, { reset: false }) : goto(to.href));
			q = mirrorOf(page.url) ?? '';
		},
		(busy) => (searching = busy),
		() => page.url.href
	);
	/** Escape gives the box back its mirrored text (a search field would clear itself) and leaves it; a pending lookup
	 *  is dropped with it. */
	function searchKey(e: KeyboardEvent) {
		if (e.key !== 'Escape') return;
		e.preventDefault();
		headerBox.cancel();
		q = mirrorOf(page.url) ?? '';
		search?.blur();
	}

	/** "/" focuses search, except where it is typed as text or a dialog is open. */
	function shortcut(e: KeyboardEvent) {
		if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return;
		const t = e.target;
		if (!(t instanceof Element) || (t as HTMLElement).isContentEditable || e.isComposing) return;
		if (t.closest('input, textarea, select, dialog[open], [role="menu"]')) return;
		if (!search) return;
		e.preventDefault();
		search.focus();
	}

	/** `?story=KEY` opens the story drawer over whatever view is showing. */
	const drawerKey = $derived(page.url.searchParams.get('story'));

	// Focus returns to whatever opened the drawer (a card's title link, a row's title) when it closes. Moving from one
	// story to another (a blocker link) remounts the drawer but keeps the first opener. Captured in onNavigate: the
	// router's default focus reset blurs the clicked link after these callbacks and before `page.url` changes, so an
	// effect on the URL would only see `body`. Restored once the drawer is gone, and only when focus went with it (focus
	// the user moved elsewhere is theirs). A drawer opened by loading its URL has no opener.
	let opener: HTMLElement | null = null;
	onNavigate(({ from, to }) => {
		// A filed-report notice outlives the dialog; only opening its story (a drawer-only change) keeps it.
		if (!(from && to && sameApartFromStory(from.url, to.url))) {
			notice = null;
			restoredNotice = false;
		}
		if (!to?.url.searchParams.has('story') || from?.url.searchParams.has('story')) return;
		const a = document.activeElement;
		opener = a instanceof HTMLElement && a !== document.body ? a : null;
	});
	function sameApartFromStory(a: URL, b: URL): boolean {
		const strip = (u: URL) => {
			const c = new URL(u.href);
			c.searchParams.delete('story');
			c.hash = '';
			return c.href;
		};
		return strip(a) === strip(b);
	}
	// A hit viewed from the report dialog unmounts with the dialog, so the recorded opener is gone: the header's Report
	// button takes the focus instead of letting it drop to the body.
	let reportButton: HTMLButtonElement | undefined = $state();
	let drawerWasOpen = false;
	$effect.pre(() => {
		const open = drawerKey !== null;
		if (!open && drawerWasOpen) {
			const o = opener;
			opener = null;
			tick().then(() => {
				if (!focusIsFree(null)) return;
				if (o?.isConnected) o.focus();
				else if (o) reportButton?.focus();
			});
		}
		drawerWasOpen = open;
	});

	/** Closing drops the parameter in place: no new history entry, scroll and focus left alone. */
	function closeDrawer() {
		const url = new URL(page.url.href);
		url.searchParams.delete('story');
		goto(url, { replace: true, reset: false });
	}

	// One event stream per tab while signed in. When the browser gives up on it (the server refused the reconnect), ask
	// `/me`: a dead session answers 401 and `unwrap` sends the tab to sign in; a live one means the server was down, so
	// try again a little later.
	const signedIn = $derived(!signedOutPage && session.me !== null);
	$effect(() => {
		if (!signedIn) return;
		return followAccess();
	});
	$effect(() => {
		if (!signedIn) return;
		let stopped = false;
		let retry: ReturnType<typeof setTimeout> | undefined;
		let dispose = () => {};
		const open = () => {
			dispose = connect(async () => {
				try {
					await unwrap(api.GET('/api/v1/me'));
				} catch {
					// the server is unreachable; the retry below covers it
				}
				if (!stopped) retry = setTimeout(open, 5000);
			});
		};
		open();
		return () => {
			stopped = true;
			clearTimeout(retry);
			dispose();
		};
	});

	// Report from anywhere: the draft lives in #lib/report.svelte.ts; this is its button, notice and sign-in restore.
	let notice = $state<{ story: Story; epicTitle: string | null } | null>(null);
	const label = $derived(reportLabel());
	let restoredNotice = $state(false);
	let restoredFor: string | null = null;
	const noticeHref = $derived.by(() => {
		if (!notice) return '';
		const url = new URL(page.url.href);
		url.searchParams.set('story', notice.story.key);
		return url.pathname + url.search;
	});

	let opening = $state(false);
	async function openFromHere() {
		if (opening) return;
		const from = page.url.href;
		const { epicKey, ...ctx } = reportContext(page.url);
		let epic: EpicRef | null = null;
		// The chip is for epic pages only, and only a new draft needs it (a kept draft keeps its own context).
		if (epicKey && !hasDraft()) {
			opening = true;
			try {
				const e = await unwrap(api.GET('/api/v1/epics/{key}', { params: { path: { key: epicKey } } }));
				epic = { key: e.key, title: e.title };
			} catch {
				// no chip: the report goes to triage
			} finally {
				opening = false;
			}
			// The reporter moved on while the epic was read: the context no longer matches the page.
			if (page.url.href !== from) return;
		}
		openReport({ ...ctx, epic });
	}

	function filed(s: Story, epicTitle: string | null) {
		restoredNotice = false;
		notice = { story: s, epicTitle };
	}

	function viewStory(k: string) {
		const url = new URL(page.url.href);
		url.searchParams.set('story', k);
		goto(url, { reset: false });
	}

	// Sign-in: bring back this tab's draft once per account; then keep it saved while it changes. Declared in this
	// order so the restore runs before the first save can clear what is stored.
	$effect(() => {
		const email = session.me?.email;
		if (!email || restoredFor === email) return;
		restoredFor = email;
		untrack(() => {
			if (restoreDraft(email)) restoredNotice = true;
		});
	});
	// The restored line is about the draft: it goes with it.
	$effect(() => {
		if (!report.draft) restoredNotice = false;
	});
	$effect(() => {
		const email = session.me?.email;
		JSON.stringify(report.draft);
		if (email && restoredFor === email) persist(email);
	});

	async function signOut() {
		discardDraft(); // a deliberate sign-out ends the draft; a session expiry (401) keeps it
		await fetch('/auth/logout', { method: 'POST', headers: { 'X-Reeve-Client': 'web' } });
		location.href = '/auth/signin';
	}
</script>

<svelte:window onkeydown={shortcut} />

{#if signedOutPage}
	{@render children()}
{:else}
	{#await load()}
		<p class="status">Loading…</p>
	{:then}
		<header class="top-bar">
			<a class="brand" href="/"><Logo />Reeve</a>
			<nav aria-label="Main">
				{#each links as link (link.href)}
					<a href={link.href} aria-current={isCurrent(page.url, link.href) ? 'page' : undefined}>{link.label}</a>
				{/each}
			</nav>
			<div class="right">
				<GoalPicker />
				<button type="button" class="primary report" bind:this={reportButton} aria-label={label} aria-busy={opening} onclick={openFromHere}>
					<span class="plus" aria-hidden="true">+</span><span class="word" aria-hidden="true">{label}</span>
				</button>
				<form role="search" aria-label="Site search" aria-busy={searching} onsubmit={(e) => { e.preventDefault(); void headerBox.submit(q, new URL(page.url.href)); }}>
					<input bind:this={search} bind:value={q} type="search" placeholder="Search or jump to a key" aria-label="Search" onkeydown={searchKey} />
					<kbd aria-hidden="true">/</kbd>
				</form>
				<a class="live" class:on={live.connected} href="/activity" title={live.connected ? 'Live: changes appear as they happen' : 'Not connected: reconnecting'}>
					<span class="dot" aria-hidden="true"></span><span class="visually-hidden">{live.connected ? 'live' : 'offline'}</span><span class="visually-hidden">, activity</span>
				</a>
				<ActionsMenu label="Account: {session.me?.email ?? ''}" items={accountItems} class="account">
					{#snippet trigger()}<Avatar email={session.me?.email} />{/snippet}
				</ActionsMenu>
			</div>
		</header>
		{#if restoredNotice}
			<div class="notices"><Notice ondismiss={() => (restoredNotice = false)}>Your report draft was restored.</Notice></div>
		{/if}
		{#if notice}
			{@const n = notice}
			<div class="notices">
				<Notice ondismiss={() => (notice = null)}>
					Filed {n.story.kind} <a href={noticeHref}>{n.story.key}</a>{#if n.story.epic}{' '}under {n.story.epic}{#if n.epicTitle}&nbsp;{n.epicTitle}{/if}.{:else}. It's waiting for the team to place it; you won't get a notification, so check {n.story.key} for updates.{/if}
					<button type="button" onclick={openFromHere}>Report another</button>
				</Notice>
			</div>
		{/if}
		<main>{@render children()}</main>
		<ReportDialog oncreated={filed} onviewstory={viewStory} />
		{#if drawerKey}
			{#key drawerKey}
				<StoryDrawer storyKey={drawerKey} onclose={closeDrawer} />
			{/key}
		{/if}
	{:catch e}
		<p class="status" role="alert">Could not load your session: {errorMessage(e)}</p>
	{/await}
{/if}

<style>
	.brand {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		color: var(--foreground);
		font-size: 18px;
		font-weight: 700;
		letter-spacing: -0.01em;
		text-decoration: none;
	}
	nav {
		display: flex;
		gap: 4px;
		font-size: 15px;
	}
	nav a {
		padding: 6px 10px;
		border-radius: 4px;
		color: var(--muted);
		font-weight: 500;
		text-decoration: none;
	}
	nav a:hover,
	nav a[aria-current='page'] {
		background: var(--lighter-background);
		color: var(--foreground);
	}
	.right {
		display: flex;
		align-items: center;
		gap: 16px;
		margin-left: auto;
		flex: 1 1 auto;
		min-width: 0;
		justify-content: flex-end;
	}
	form[role='search'] {
		position: relative;
		flex: 1 1 auto;
		width: 120px;
		max-width: 240px;
		min-width: 0;
	}
	form[role='search'] input {
		width: 100%;
		padding-right: 28px;
	}
	/* A key lookup is pending: the box stays usable (a new submit supersedes it) but reads as working. */
	form[role='search'][aria-busy='true'] input {
		color: var(--muted);
		cursor: progress;
	}
	kbd {
		position: absolute;
		top: 50%;
		right: 8px;
		transform: translateY(-50%);
		padding: 0 5px;
		border: 1px solid var(--line);
		border-radius: 4px;
		color: var(--muted);
		font-family: var(--mono);
		font-size: 13px;
	}
	.notices {
		padding: 12px 20px 0;
	}
	.report .plus {
		display: none;
	}
	@media (max-width: 720px) {
		.notices {
			padding: 12px 16px 0;
		}
		nav {
			flex-wrap: wrap;
			min-width: 0;
		}
		.right {
			flex-wrap: wrap;
			gap: 8px 12px;
		}
		form[role='search'] {
			flex: 1 1 120px;
			min-width: 80px;
		}
		.report .plus {
			display: inline;
			font-weight: 700;
		}
		.report .word {
			position: absolute;
			width: 1px;
			height: 1px;
			overflow: hidden;
			clip-path: inset(50%);
			white-space: nowrap;
		}
	}
	.live {
		display: grid;
		place-items: center;
		width: 24px;
		height: 24px;
	}
	.live .dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--muted);
	}
	.live.on .dot {
		background: var(--ready);
	}
	:global(.menu-trigger.account) {
		width: 30px;
		height: 30px;
		padding: 0;
		border-radius: 50%;
		justify-content: center;
		border-color: var(--line);
	}
</style>
