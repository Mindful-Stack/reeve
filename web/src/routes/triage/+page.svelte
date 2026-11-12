<!-- web/src/routes/triage/+page.svelte: the untriaged queue (kb/ADR-0018 R4), oldest first, one item at a time. The server
	filters (GET /stories?untriaged=true, kb/ADR-0021 R1); this page only sorts. Keys act only while focus is inside the
	page's triage area (WCAG 2.1.4), and never while a drawer is open over it. -->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, Conflict, unwrap } from '#lib/api/client.ts';
	import type { EditItem, Epic, Story } from '#lib/api/types.ts';
	import { COLUMN_LABEL, KINDS } from '#lib/board.ts';
	import TriageItem from '#lib/components/TriageItem.svelte';
	import { storyHref } from '#lib/drawer.ts';
	import { onLive } from '#lib/events.svelte.ts';
		import { errorMessage, plural } from '#lib/format.ts';
	import { postMove } from '#lib/moves.ts';
	import {
		ARM_MS,
		armedFor,
		byKey,
		draftOf,
		FIELD_LABEL,
		followDraft,
		nextKey,
		placeEdit,
		rebaseAfterConflict,
		undoEdit,
		whereNow,
		type Arm,
		type Draft
	} from '#lib/triage.ts';
	import { command, type Command } from '#lib/triageKeys.ts';
	import PageHeader from '#lib/ui/PageHeader.svelte';

	/** The one line of news: the outcome of the last action, with a link to the item it names. The next message replaces
	 *  it. `error`: a failure or a conflict, which a background announcement never covers. */
	type Notice = { text: string; open?: string; error?: boolean };

	/** What Undo reverts: the action, the item it writes to, and the write. */
	type Undo = { action: 'place' | 'drop'; key: string; run: () => Promise<void> };

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
	/** The one write in flight (its item's key), or null. Every write waits for it. */
	let writing = $state<string | null>(null);
	/** The last Place's or Drop's Undo, apart from the notice so that no other message ends it: it lasts until it is
	 *  used or the next Place or Drop replaces it. */
	let undoable = $state.raw<Undo | null>(null);
	let arm = $state<Arm | null>(null);
	let armTimer: ReturnType<typeof setTimeout> | undefined;
	const armed = $derived(
		arm && current && arm.key === current.key && arm.version === drafts[current.key]?.base.version
			? armText(current, drafts[current.key].duplicate)
			: null
	);

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
		else if (!left && writing !== currentKey && !queue.some((s) => s.key === currentKey)) await loadLeft(currentKey, n);
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

	/** Back to the queue after a write or a live change disabled the form, unless the user took focus elsewhere: to a
	 *  control outside the item form (the Keys toggle, a link in the top bar) or into a field (another item's, during a
	 *  slow write: its letters must not become commands). Free is the page itself, a disabled control that gave focus up,
	 *  the queue itself, or the item form being written. */
	function focusBack(): void {
		const a = document.activeElement;
		const form = area?.querySelector('form.place');
		const free = !a || a === document.body || a === list || a.matches(':disabled') || !!form?.contains(a);
		if (free && !typingIn(a)) void focusQueue();
	}

	/** Typing in an enabled field. A field disabled under the focus (the item left, a write is in flight) is not typing,
	 *  so `j` still moves on. */
	function typingIn(el: EventTarget | null): boolean {
		if (!(el instanceof HTMLElement)) return false;
		if (el.isContentEditable) return true;
		return (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) && !el.disabled;
	}

	function armText(s: Story, dup: string | null): string {
		const stop = s.delivery === 'todo' ? '' : ' and stop active work (Undo returns it as todo)';
		return `Drop ${s.key}${dup ? ` as a duplicate of ${dup}` : ''}${stop}? Press d or click again.`;
	}
	function disarm(): void {
		clearTimeout(armTimer);
		arm = null;
	}

	function run(c: Command): void {
		switch (c.kind) {
			case 'next':
			case 'prev':
				step(c.kind === 'next' ? 1 : -1);
				return void focusQueue();
			case 'skip':
				return skip();
			case 'place':
				return void place();
			case 'drop':
				return void drop();
			case 'undo':
				return void undo();
			case 'epic':
				return void area?.querySelector<HTMLElement>('.epic [role="combobox"]')?.focus();
			case 'criteria':
				return void area?.querySelector<HTMLElement>('textarea.criteria')?.focus();
			case 'kind': {
				const d = currentKey ? drafts[currentKey] : undefined;
				if (!d || left || writing === currentKey) return;
				d.fields.kind = KINDS[(KINDS.indexOf(d.fields.kind) + 1) % KINDS.length];
				return void say({ text: `Kind: ${d.fields.kind}.` });
			}
			case 'open':
				if (currentKey) void goto(storyHref(page.url, currentKey), { reset: false });
				return;
			case 'escape':
				if (typingIn(document.activeElement)) area?.querySelector<HTMLElement>('#triage-item-title')?.focus();
				if (arm) {
					disarm();
					void say({ text: 'Drop cancelled.' });
				}
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

	/** An action finished for `key`: it leaves the queue, and the selection moves on only if the user is still on it. */
	function finished(key: string, order: string[], n: Notice, u: Undo): void {
		loads++; // a reload in flight read the queue before this write: drop it; the write's own live refetch follows
		queue = queue.filter((s) => s.key !== key);
		delete drafts[key];
		if (currentKey === key) select(nextKey(order, key));
		undoable = u;
		void say(n);
	}

	/** Undo brought `s` back: it rejoins the queue here, as `finished()` took it out, and becomes current. */
	function returned(s: Story): void {
		loads++; // a reload in flight read the queue without it; the write's own live refetch follows
		queue = byKey([...queue.filter((x) => x.key !== s.key), s]);
		select(s.key);
	}

	async function place(): Promise<void> {
		const s = current;
		if (!s || left || writing) return;
		const d = drafts[s.key];
		const edit = placeEdit(d);
		if (edit.kind === 'invalid') {
			if (!d.fields.epic) area?.querySelector<HTMLElement>('.epic [role="combobox"]')?.focus();
			void say({ text: edit.error, error: true });
			return;
		}
		const order = queue.map((x) => x.key);
		writing = s.key;
		try {
			const placed = await unwrap(api.PATCH('/api/v1/stories/{key}', { params: { path: { key: s.key } }, body: edit.body }));
			finished(s.key, order, { text: `Placed ${s.key} under ${placed.epic}.`, open: s.key }, placeUndo(placed, d, edit.body));
		} catch (e) {
			if (e instanceof Conflict) await conflicted(s.key, d, 'place');
			else void say({ text: `${s.key} was not placed: ${errorMessage(e)}`, error: true });
		} finally {
			writing = null;
			focusBack();
		}
	}

	/** After a 409: read the item, keep the user's fields, take the server's others, and name the fields both changed.
	 *  `head` says what landed before the conflict (a duplicate comment). */
	async function conflicted(key: string, d: Draft, again: 'place' | 'drop', head = ''): Promise<void> {
		try {
			const server = await unwrap(api.GET('/api/v1/stories/{key}', { params: { path: { key } } }));
			if (server.epic || server.delivery === 'done') {
				// No longer untriaged (kb/ADR-0018 R4): it leaves the queue and the page says where it went, as on a live change.
				queue = queue.filter((x) => x.key !== key);
				if (currentKey === key) left = server;
				return void say({ text: `${head}${whereNow(server)}`, error: true });
			}
			const { draft, clashes } = rebaseAfterConflict(d, server);
			drafts[key] = draft;
			queue = queue.map((x) => (x.key === key ? server : x));
			const named = clashes.map((c) => `${FIELD_LABEL[c.field]} is now "${c.value}"`).join('; ');
			void say({ text: `${head}${key} changed meanwhile${named ? `: ${named}` : ''}. Check and ${again} again.`, error: true });
		} catch (e) {
			void say({ text: `${head}${key} changed meanwhile and could not be reloaded: ${errorMessage(e)}`, error: true });
		}
	}

	async function drop(): Promise<void> {
		const s = current;
		if (!s || left || writing) return;
		const d = drafts[s.key];
		const version = d.base.version;
		const dup = d.duplicate;
		if (!armedFor(arm, s.key, version, Date.now())) {
			disarm();
			const prompt = armText(s, dup);
			arm = { key: s.key, version, at: Date.now() };
			armTimer = setTimeout(() => {
				arm = null;
				if (notice?.text === prompt) void say({ text: 'Drop cancelled.' });
			}, ARM_MS);
			void say({ text: prompt });
			return;
		}
		disarm();
		const order = queue.map((x) => x.key);
		const failed = dup ? 'Comment saved; drop failed: ' : '';
		writing = s.key;
		try {
			const out = await postMove({ key: s.key, version }, 'done', { resolution: 'dropped' });
			if (out.kind !== 'moved') {
				void say({ text: `${failed || `${s.key} was not dropped: `}${out.kind === 'notice' ? out.notice : 'refused'}`, error: true });
				return;
			}
			finished(s.key, order, { text: `Dropped ${s.key}${dup ? ` as a duplicate of ${dup}` : ''}.`, open: s.key }, dropUndo(out.story, !!dup));
		} catch (e) {
			if (e instanceof Conflict) await conflicted(s.key, d, 'drop', failed);
			else void say({ text: `${failed || `${s.key} was not dropped: `}${errorMessage(e)}`, error: true });
		} finally {
			writing = null;
			focusBack();
		}
	}

	function placeUndo(placed: Story, d: Draft, sent: EditItem): Undo {
		return {
			action: 'place',
			key: placed.key,
			run: async () => {
				const back = await unwrap(api.PATCH('/api/v1/stories/{key}', { params: { path: { key: placed.key } }, body: undoEdit(placed, d, sent) }));
				const restored = draftOf(back);
				restored.fields = { ...d.fields, epic: '' };
				drafts[back.key] = restored; // before select(), which keeps an existing draft: the typed fields come back
				returned(back);
				void say({ text: `Undone: ${back.key} is back in the queue.` });
			}
		};
	}

	/** Undo of a Drop always moves the item to todo, whatever it was: todo is not active work, so no readiness gate
	 *  applies (kb/ADR-0018 R2) and Undo never needs an override. */
	function dropUndo(dropped: Story, duplicate: boolean): Undo {
		return {
			action: 'drop',
			key: dropped.key,
			run: async () => {
				const out = await postMove({ key: dropped.key, version: dropped.version }, 'todo');
				if (out.kind !== 'moved') throw new Error(`${dropped.key} could not be moved back to todo`);
				returned(out.story);
				void say({ text: `Undone: ${dropped.key} is back in todo.${duplicate ? ' The duplicate comment stays.' : ''}` });
			}
		};
	}

	async function undo(): Promise<void> {
		const u = undoable;
		if (!u || writing) return;
		undoable = null; // one Undo per action
		writing = u.key;
		try {
			await u.run();
		} catch (e) {
			const text = e instanceof Conflict ? `${u.key} changed since; undo it in the drawer.` : `Undo failed: ${errorMessage(e)}`;
			void say({ text, open: u.key, error: true });
		} finally {
			writing = null;
			focusBack();
		}
	}

	onMount(() => {
		void reload();
		return () => clearTimeout(armTimer);
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
		{#if undoable}<button type="button" onclick={undo}>Undo {undoable.action} {undoable.key} <kbd>u</kbd></button>{/if}
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
					disabled={!!left || writing === current.key}
					gone={left ? whereNow(left) : null}
					open={storyHref(page.url, current.key)}
					onplace={place}
					{armed}
					ondrop={drop}
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
