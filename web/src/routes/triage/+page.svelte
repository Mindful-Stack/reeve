<!-- web/src/routes/triage/+page.svelte: the untriaged queue (kb/ADR-0018 R4), oldest first, one item at a time. The server
	filters (GET /stories?untriaged=true, kb/ADR-0021 R1); this page only sorts. Keys act only while focus is inside the
	page's triage area (WCAG 2.1.4), and never while a drawer is open over it. -->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { Epic, Story } from '#lib/api/types.ts';
	import { COLUMN_LABEL, KINDS } from '#lib/board.ts';
	import TriageItem from '#lib/components/TriageItem.svelte';
	import { storyHref } from '#lib/drawer.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { errorMessage, plural } from '#lib/format.ts';
	import { byKey, draftOf, followDraft, whereNow, type Draft } from '#lib/triage.ts';
	import { command, type Command } from '#lib/triageKeys.ts';
	import PageHeader from '#lib/ui/PageHeader.svelte';

	/** The one line of news: the outcome of the last action, with a link to the item it names. The next message replaces
	 *  it. `error`: a failure or a conflict, which a background announcement never covers. */
	type Notice = { text: string; open?: string; error?: boolean };

	let queue = $state.raw<Story[]>([]);
	let epics = $state.raw<Epic[]>([]);
	let loaded = $state(false);
	let loadError = $state<string | null>(null);
	let currentKey = $state<string | null>(null);
	/** The current item when it left the queue meanwhile (read directly), else null. */
	let left = $state<Story | null>(null);
	/** Each visited item's draft, by key: deeply reactive, so `t` (which changes a field inside a draft) re-renders. */
	let drafts = $state<Record<string, Draft>>({});
	let notice = $state.raw<Notice | null>(null);
	let help = $state(false);
	let area: HTMLElement | undefined = $state();
	let list: HTMLElement | undefined = $state();

	const current = $derived(left ?? queue.find((s) => s.key === currentKey) ?? null);
	const position = $derived(queue.findIndex((s) => s.key === currentKey));

	// The current option stays in sight in a scrolled queue (jsdom has no scrollIntoView, hence the optional call).
	$effect(() => {
		if (currentKey) document.getElementById(`triage-opt-${currentKey}`)?.scrollIntoView?.({ block: 'nearest' });
	});

	/** Empties the live region first, so the same message is read again. */
	async function say(n: Notice) {
		notice = null;
		await tick();
		notice = n;
	}

	let loads = 0;
	async function reload(): Promise<void> {
		const n = ++loads;
		let rows, eps;
		try {
			[rows, eps] = await Promise.all([
				unwrap(api.GET('/api/v1/stories', { params: { query: { untriaged: true } } })),
				unwrap(api.GET('/api/v1/epics'))
			]);
		} catch (e) {
			if (n === loads) loadError = errorMessage(e);
			return;
		}
		if (n !== loads) return; // a newer reload answers instead
		loadError = null;
		const before = new Set(queue.map((s) => s.key));
		queue = byKey(rows.map((r) => r.story));
		epics = eps;
		for (const s of queue) if (s.key in drafts) drafts[s.key] = followDraft(drafts[s.key], s);
		const added = loaded ? queue.filter((s) => !before.has(s.key)).length : 0;
		// Never over what the user must still read: an error or a conflict.
		if (added && !notice?.error) void say({ text: `${plural(added, 'new untriaged item', 'new untriaged items')}.` });
		const first = !loaded;
		loaded = true;
		if (currentKey === null) select(queue[0]?.key ?? null);
		// Gone from the queue: read it to say where it went.
		else if (!left && !queue.some((s) => s.key === currentKey)) await loadLeft(currentKey, n);
		// Focus on entry comes with the first answer that lands, whichever reload sent it.
		if (first) void focusQueue();
	}

	async function loadLeft(key: string, n: number): Promise<void> {
		try {
			const s = await unwrap(api.GET('/api/v1/stories/{key}', { params: { path: { key } } }));
			if (n !== loads || currentKey !== key) return;
			left = s;
			if (!notice?.error) void say({ text: whereNow(s) });
			focusBack(); // the pane lost the item until this read landed, and its form is disabled now: focus may be on the page
		} catch {
			if (n === loads && currentKey === key) select(queue[0]?.key ?? null);
		}
	}

	function select(key: string | null): void {
		currentKey = key;
		left = null;
		const s = queue.find((x) => x.key === key);
		if (s && !(s.key in drafts)) drafts[s.key] = draftOf(s);
	}

	function step(by: 1 | -1): void {
		if (!queue.length) return;
		const i = position < 0 ? 0 : Math.min(queue.length - 1, Math.max(0, position + by));
		select(queue[i].key);
	}

	/** Skip writes nothing; it wraps from the last item to the first, where `j` stops. */
	function skip(): void {
		if (!queue.length) return;
		const from = currentKey;
		const i = (position + 1) % queue.length;
		select(queue[i].key);
		void say({ text: i === 0 ? `Skipped ${from}; back to the first item.` : `Skipped ${from}.` });
		void focusQueue();
	}

	/** Focus goes back to the queue: its listbox (the current option is its active descendant), or the item's heading
	 *  when the list cannot take focus (hidden on a phone); the empty state's heading when nothing is left. */
	async function focusQueue(): Promise<void> {
		await tick();
		if (!queue.length) {
			const empty = area?.querySelector<HTMLElement>('#triage-empty');
			return void (empty ?? area?.querySelector<HTMLElement>('#triage-item-title'))?.focus();
		}
		list?.focus();
		if (document.activeElement !== list) area?.querySelector<HTMLElement>('#triage-item-title')?.focus();
	}

	/** Back to the queue after a write or a live change disabled the form, unless the user moved focus out of the triage
	 *  area or is typing in a field (another item's, during a slow write: its letters must not become commands). */
	function focusBack(): void {
		if (focusIsFree(area) && !typingIn(document.activeElement)) void focusQueue();
	}

	/** Typing in an enabled field. A field disabled under the focus (the item left, a write is in flight) is not typing,
	 *  so `j` still moves on. */
	function typingIn(el: EventTarget | null): boolean {
		if (!(el instanceof HTMLElement)) return false;
		if (el.isContentEditable) return true;
		return (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) && !el.disabled;
	}

	function run(c: Command): void {
		switch (c.kind) {
			case 'next':
			case 'prev':
				step(c.kind === 'next' ? 1 : -1);
				return void focusQueue();
			case 'skip':
				return skip();
			case 'epic':
				return void area?.querySelector<HTMLElement>('.epic [role="combobox"]')?.focus();
			case 'criteria':
				return void area?.querySelector<HTMLElement>('textarea.criteria')?.focus();
			case 'kind': {
				const d = currentKey ? drafts[currentKey] : undefined;
				if (!d || left) return;
				d.fields.kind = KINDS[(KINDS.indexOf(d.fields.kind) + 1) % KINDS.length];
				return void say({ text: `Kind: ${d.fields.kind}.` });
			}
			case 'open':
				if (currentKey) void goto(storyHref(page.url, currentKey), { reset: false });
				return;
			case 'escape':
				if (typingIn(document.activeElement)) area?.querySelector<HTMLElement>('#triage-item-title')?.focus();
				return;
			case 'help':
				help = !help;
				return;
		}
	}

	function keydown(e: KeyboardEvent): void {
		const q = page.url.searchParams;
		const c = command(e, { typing: typingIn(e.target), inList: e.target === list, overlay: q.has('story') || q.has('decision') });
		if (!c) return;
		e.preventDefault();
		run(c);
	}

	onMount(() => {
		void reload();
	});
	onLive(reload);
</script>

<PageHeader title="Triage" />
<!-- The keys are this area's, not the page's: a keydown from anywhere else never reaches this handler. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="triage" bind:this={area} onkeydown={keydown}>
	<div class="notice">
		<p role="status" aria-live="polite" data-testid="triage-status">{notice?.text ?? ''}</p>
		{#if notice?.open}<a href={storyHref(page.url, notice.open)} data-sveltekit-reset="false">Open {notice.open}</a>{/if}
	</div>
	{#if loadError}<p role="alert" class="error">Couldn't load the queue: {loadError}</p>{/if}
	{#if loaded && !queue.length && !left && currentKey === null}
		<div class="empty">
			<h2 id="triage-empty" tabindex="-1">Nothing to triage</h2>
			<p><a href="/backlog">Go to the backlog</a></p>
		</div>
	{:else if loaded}
		<ul
			class="queue"
			role="listbox"
			aria-label="Untriaged"
			tabindex="0"
			bind:this={list}
			aria-activedescendant={currentKey && position >= 0 ? `triage-opt-${currentKey}` : undefined}
		>
			{#each queue as s, i (s.key)}
				<!-- svelte-ignore a11y_click_events_have_key_events -->
				<li
					role="option"
					id="triage-opt-{s.key}"
					data-key={s.key}
					aria-selected={s.key === currentKey}
					aria-posinset={i + 1}
					aria-setsize={queue.length}
					onclick={() => select(s.key)}
				>
					<span class="key">{s.key}</span> {s.title}{#if s.delivery !== 'todo'}<span class="state"> · {COLUMN_LABEL[s.delivery]}</span>{/if}
				</li>
			{/each}
		</ul>
		<div class="pane">
			<p class="counter">{#if position >= 0}{position + 1} of {queue.length}{/if}
				<button type="button" onclick={() => step(-1)}>Previous</button>
				<button type="button" onclick={() => step(1)}>Next</button>
				<button type="button" aria-expanded={help} onclick={() => (help = !help)}>Keys (?)</button>
			</p>
			{#if help}
				<dl class="keys">
					<dt>j / k</dt><dd>next / previous (stops at the ends)</dd><dt>e</dt><dd>epic (Enter on a new epic moves on to criteria)</dd>
					<dt>t</dt><dd>kind</dd><dt>c</dt><dd>criteria</dd><dt>Ctrl+Enter</dt><dd>place</dd><dt>d, d</dt><dd>drop</dd>
					<dt>1–5</dt><dd>mark a similar item as the duplicate</dd><dt>s</dt><dd>skip (after the last, back to the first)</dd>
					<dt>o</dt><dd>open in the drawer</dd><dt>u</dt><dd>undo</dd><dt>Escape</dt><dd>leave a field, cancel a drop</dd>
				</dl>
			{/if}
			{#if current && current.key in drafts}
				<TriageItem
					item={current}
					bind:draft={drafts[current.key]}
					{epics}
					disabled={!!left}
					gone={left ? whereNow(left) : null}
					open={storyHref(page.url, current.key)}
					onplace={() => {}}
					ondrop={() => {}}
					onskip={skip}
				/>
			{/if}
		</div>
	{/if}
</div>

<style>
	.triage { display: grid; grid-template-columns: minmax(220px, 1fr) 3fr; gap: 16px; }
	/* Sticky under the top bar, so the outcome and its Undo stay in sight on a long pane (a phone above all). */
	.notice {
		grid-column: 1 / -1;
		position: sticky;
		top: var(--top-bar-height, 0px);
		z-index: 1;
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: baseline;
		min-height: 1.4em;
		background: var(--background);
	}
	.notice p { margin: 0; }
	.error { grid-column: 1 / -1; color: var(--danger); }
	.queue { list-style: none; margin: 0; padding: 0; overflow-y: auto; max-height: calc(100vh - var(--top-bar-height, 0px) - 120px); }
	.queue li { padding: 6px 8px; cursor: pointer; border-radius: 6px; }
	.queue li[aria-selected='true'] { background: var(--selection); outline: 2px solid var(--accent); }
	.queue:focus-visible { outline: 2px solid var(--accent); }
	.state { color: var(--muted); }
	.pane { min-width: 0; }
	.counter :global(button) { margin-left: 8px; }
	@media (max-width: 719px) {
		.triage { grid-template-columns: 1fr; }
		.queue { display: none; }
	}
</style>
