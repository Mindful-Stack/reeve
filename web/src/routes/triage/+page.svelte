<!-- web/src/routes/triage/+page.svelte: the untriaged queue (kb/ADR-0018 R4), oldest first, one item at a time. The server
	filters (GET /stories?untriaged=true, kb/ADR-0021 R1); this page only sorts. Keys act only while focus is inside the
	page's triage area (WCAG 2.1.4), and never while a drawer is open over it. -->
<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, Conflict, unwrap } from '#lib/api/client.ts';
	import type { EditItem, Epic, SimilarStory, Story } from '#lib/api/types.ts';
	import { COLUMN_LABEL, fullLabel, KINDS } from '#lib/board.ts';
	import TriageItem from '#lib/components/TriageItem.svelte';
	import { hasDrawer, storyHref } from '#lib/drawer.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { errorMessage } from '#lib/format.ts';
	import { postMove, type Refusal } from '#lib/moves.ts';
	import { SIMILAR_SHOWN, similarQuery, withoutSelf } from '#lib/similar.ts';
	import {
		byKey,
		draftOf,
		duplicateCommentKey,
		duplicateText,
		FIELD_LABEL,
		followDraft,
		nextKey,
		placeEdit,
		rebaseAfterConflict,
		undoEdit,
		whereNow,
		type Draft
	} from '#lib/triage.ts';
	import { ACTIVE } from '#lib/sprint.ts';
	import { command, type Command } from '#lib/triageKeys.ts';
	import PageHeader from '#lib/ui/PageHeader.svelte';

	/** The one line of news: the outcome of the last action, with a link to the item it names. The next message replaces
	 *  it; a failure's words begin with what failed. */
	type Notice = { text: string; open?: string };

	/** What Undo reverts: the action, the item it writes to, and the write. */
	type Undo = { action: 'place' | 'drop'; key: string; run: () => Promise<void> };

	/** The drop armed by a first `d`: the item and the version it was reviewed at. Leaving the item, marking a duplicate
	 *  or Escape ends it. */
	type Arm = { key: string; version: number };

	let queue = $state.raw<Story[]>([]);
	let epics = $state.raw<Epic[]>([]);
	let loaded = $state(false);
	let loadError = $state<string | null>(null);
	let currentKey = $state<string | null>(null);
	/** The current item when it left the queue meanwhile (read directly), else null. */
	let left = $state.raw<Story | null>(null);
	/** Each visited item's draft, by key: deeply reactive, so `t` (which changes a field inside a draft) re-renders. */
	let drafts = $state<Record<string, Draft | undefined>>({});
	let notice = $state.raw<Notice | null>(null);
	let help = $state(false);
	let area: HTMLElement | undefined = $state();
	let list: HTMLElement | undefined = $state();
	let emptyEl: HTMLElement | undefined = $state();
	let pane: ReturnType<typeof TriageItem> | undefined = $state();

	const current = $derived(left ?? queue.find((s) => s.key === currentKey) ?? null);
	const draft = $derived(current ? drafts[current.key] : undefined);
	const position = $derived(queue.findIndex((s) => s.key === currentKey));
	/** A write is in flight. Every action waits for it, whichever item it is on, and every item's form says so. */
	let busy = $state(false);
	/** The last Place's or Drop's Undo, apart from the notice so that no other message ends it: it lasts until it is
	 *  used or the next Place or Drop replaces it. */
	let undoable = $state.raw<Undo | null>(null);
	let arm = $state.raw<Arm | null>(null);
	/** The current item's similar stories; empty until they land, and for a title outside what the server compares. */
	let similar = $state.raw<SimilarStory[]>([]);
	const armed = $derived(!!arm && !!current && arm.key === current.key && arm.version === draft?.base.version);

	// The current option stays in sight in a scrolled queue (jsdom has no scrollIntoView, hence the optional call).
	$effect(() => {
		if (currentKey) document.getElementById(`triage-opt-${currentKey}`)?.scrollIntoView?.({ block: 'nearest' });
	});


	let loads = 0;
	/** A write patched the queue here: a reload that read the list before it answers stale, so its answer is dropped. */
	const dropPendingReload = () => loads++;

	async function reload(): Promise<void> {
		const n = ++loads;
		const got = await Promise.all([
			unwrap(api.GET('/api/v1/stories', { params: { query: { untriaged: true } } })),
			unwrap(api.GET('/api/v1/epics'))
		]).catch((e) => {
			if (n === loads) loadError = errorMessage(e);
			return null;
		});
		if (!got || n !== loads) return; // failed, or a newer reload answers instead
		const [rows, eps] = got;
		loadError = null;
		queue = byKey(rows.map((r) => r.story));
		epics = eps;
		for (const s of queue) {
			const d = drafts[s.key];
			if (d) drafts[s.key] = followDraft(d, s);
		}
		const here = queue.some((s) => s.key === currentKey);
		if (left && here) left = null; // someone put it back: it is no longer gone
		const first = !loaded;
		loaded = true;
		if (currentKey === null) select(queue[0]?.key ?? null);
		// Gone from the queue: read it to say where it went (not while a write is in flight: its answer says).
		else if (!left && !busy && !here) await loadLeft(currentKey, n);
		// Focus on entry comes with the first answer that lands, whichever reload sent it.
		if (first) void focusQueue();
	}

	async function loadLeft(key: string, n: number): Promise<void> {
		try {
			const s = await unwrap(api.GET('/api/v1/stories/{key}', { params: { path: { key } } }));
			if (n !== loads || currentKey !== key) return;
			const where = whereNow(s);
			if (where === null) return; // untriaged again: its own live event reloads the list and finds it, with the user's place kept
			left = s;
			notice = { text: where };
			focusBack(); // the pane lost the item until this read landed, and its form is disabled now: focus may be on the page
		} catch {
			if (n === loads && currentKey === key) select(queue[0]?.key ?? null);
		}
	}

	/** Loads the similar list for `s`; an answer for an item the user has left is dropped. */
	async function loadSimilar(s: Story): Promise<void> {
		const q = similarQuery(s.title);
		if (!q) {
			similar = []; // too short or too long to compare: nothing is asked
			return;
		}
		let hits: SimilarStory[] = [];
		try {
			hits = withoutSelf(await unwrap(api.GET('/api/v1/stories/similar', { params: { query: { q, limit: SIMILAR_SHOWN + 1 } } })), s.key);
		} catch {
			// shown as nothing similar
		}
		if (currentKey === s.key) similar = hits;
	}

	function select(key: string | null): void {
		if (key === currentKey && !left) return; // the same item again: nothing to load, and its hits stay put
		currentKey = key;
		left = null;
		if (arm) notice = null; // its prompt names the item just left
		arm = null; // a drop armed for the item just left is not armed for this one
		similar = [];
		const s = queue.find((x) => x.key === key);
		if (!s) return;
		if (!drafts[s.key]) drafts[s.key] = draftOf(s);
		void loadSimilar(s);
	}

	function step(by: 1 | -1): void {
		if (!queue.length) {
			select(null); // nothing left: leave the item that went, for the empty state
			return void focusQueue();
		}
		const i = position < 0 ? 0 : Math.min(queue.length - 1, Math.max(0, position + by));
		if (position >= 0 && i === position) return void (notice = { text: by > 0 ? 'Last item.' : 'First item.' });
		select(queue[i].key);
	}

	/** Skip writes nothing; it wraps from the last item to the first, where `j` stops. */
	function skip(): void {
		if (!queue.length) {
			select(null);
			return void focusQueue();
		}
		const from = currentKey;
		const i = (position + 1) % queue.length;
		select(queue[i].key);
		notice = { text: i === 0 ? `Skipped ${from}; back to the first item.` : `Skipped ${from}.` };
		void focusQueue();
	}

	/** Focus goes back to the queue: its listbox (the current option is its active descendant), or the item's heading
	 *  when the list cannot take focus (hidden on a phone); the empty state's heading when nothing is left. */
	async function focusQueue(): Promise<void> {
		await tick();
		if (!queue.length) return emptyEl ? emptyEl.focus() : pane?.focus('heading');
		list?.focus();
		if (document.activeElement !== list) pane?.focus('heading');
	}

	/** Back to the queue after a write or a live change disabled the form, unless the user is on an enabled control or
	 *  field (another item's Skip, the Keys toggle, a link in the top bar, a field whose letters must not become
	 *  commands). Free is the page itself, the item heading or body, the queue (a `ul[tabindex]`) and a disabled control.
	 *  Focus outside the triage area (the drawer's heading, a control there) is never taken. */
	function focusBack(): void {
		if (focusIsFree(area) && !document.activeElement?.matches('a[href], button:enabled, input:enabled, select:enabled, textarea:enabled')) void focusQueue();
	}

	/** Typing in an enabled field. A field disabled under the focus (the item left, a write is in flight) is not typing,
	 *  so `j` still moves on. */
	function typingIn(el: EventTarget | null): boolean {
		if (!(el instanceof HTMLElement)) return false;
		if (el.isContentEditable) return true;
		return (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) && !el.disabled;
	}

	function armText(s: Story, dup: string | null): string {
		const stop = ACTIVE.has(s.delivery) ? ' and stop active work (Undo returns it as todo)' : '';
		return `Drop ${s.key}${dup ? ` as a duplicate of ${dup}` : ''}${stop}? Press d or click again.`;
	}

	/** What a refused move says: the notice the move wrote, or every reason when an override would have been offered. */
	const why = (r: Refusal): string => (r.kind === 'notice' ? r.notice : r.reasons.map((x) => fullLabel(x)).join('; '));

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
			case 'duplicate': {
				const hit = similar[c.n - 1];
				if (hit) mark(hit.key);
				return;
			}
			case 'epic':
				return pane?.focus('epic');
			case 'criteria':
				return pane?.focus('criteria');
			case 'kind':
				if (!draft || left || busy) return;
				draft.fields.kind = KINDS[(KINDS.indexOf(draft.fields.kind) + 1) % KINDS.length];
				return void (notice = { text: `Kind: ${draft.fields.kind}.` });
			case 'open':
				if (currentKey) void goto(storyHref(page.url, currentKey), { reset: false });
				return;
			case 'escape':
				if (typingIn(document.activeElement)) pane?.focus('heading');
				if (arm) {
					arm = null;
					notice = { text: 'Drop cancelled.' };
				}
				return;
			case 'help':
				help = !help;
				return;
			default:
				c satisfies never;
		}
	}

	function keydown(e: KeyboardEvent): void {
		const c = command(e, { typing: typingIn(e.target), inList: e.target === list, overlay: hasDrawer(page.url) });
		if (!c) return;
		e.preventDefault();
		run(c);
	}

	/** Marks (or, the same hit again, unmarks) the current item as a duplicate of `hit`. A drop armed before names no
	 *  duplicate, so it is disarmed. Not while the item is gone or a write is in flight (the drop in flight named its
	 *  duplicate). */
	function mark(hit: string): void {
		if (!draft || left || busy) return;
		draft.duplicateOf = draft.duplicateOf === hit ? null : hit;
		arm = null;
		notice = { text: draft.duplicateOf ? `Marked as a duplicate of ${draft.duplicateOf}.` : 'Duplicate mark cleared.' };
	}

	async function commentDuplicate(key: string, dup: string): Promise<boolean> {
		try {
			await unwrap(
				api.POST('/api/v1/stories/{key}/comments', {
					params: { path: { key } },
					body: { body: duplicateText(dup), idempotency_key: duplicateCommentKey(key, dup) }
				})
			);
			return true;
		} catch (e) {
			notice = { text: `${key} was not dropped: the comment failed: ${errorMessage(e)}` };
			return false;
		}
	}

	/** An action finished for `key`: it leaves the queue, and the selection moves on only if the user is still on it. */
	function finished(key: string, order: string[], n: Notice, u: Undo): void {
		dropPendingReload(); // the write's own live refetch follows
		queue = queue.filter((s) => s.key !== key);
		delete drafts[key];
		if (currentKey === key) select(nextKey(order, key));
		undoable = u;
		notice = n;
	}

	/** Undo brought `s` back: it rejoins the queue here, as `finished()` took it out, and becomes current. */
	function returned(s: Story): void {
		dropPendingReload(); // the write's own live refetch follows
		queue = byKey([...queue.filter((x) => x.key !== s.key), s]);
		select(s.key);
	}

	async function place(): Promise<void> {
		const s = current;
		const d = draft;
		if (!s || !d || left || busy) return;
		const edit = placeEdit(d);
		if (edit.kind === 'invalid') {
			if (!d.fields.epic) pane?.focus('epic');
			notice = { text: edit.error };
			return;
		}
		const order = queue.map((x) => x.key);
		busy = true;
		try {
			const placed = await unwrap(api.PATCH('/api/v1/stories/{key}', { params: { path: { key: s.key } }, body: edit.body }));
			finished(s.key, order, { text: `Placed ${s.key} under ${placed.epic}.`, open: s.key }, placeUndo(placed, d, edit.body));
		} catch (e) {
			if (e instanceof Conflict) await conflicted(s.key, d, 'place');
			else notice = { text: `${s.key} was not placed: ${errorMessage(e)}` };
		} finally {
			busy = false;
			focusBack();
		}
	}

	/** After a 409: read the item, keep the user's fields, take the server's others, and name the fields both changed.
	 *  `tail` says what landed before the conflict (a duplicate comment). */
	async function conflicted(key: string, d: Draft, again: 'place' | 'drop', tail = ''): Promise<void> {
		try {
			const server = await unwrap(api.GET('/api/v1/stories/{key}', { params: { path: { key } } }));
			const where = whereNow(server);
			if (where !== null) {
				// No longer untriaged (kb/ADR-0018 R4): it leaves the queue and the page says where it went, as on a live change.
				queue = queue.filter((x) => x.key !== key);
				if (currentKey === key) left = server;
				notice = { text: `${where}${tail}` };
			} else {
				const { draft: rebased, clashes } = rebaseAfterConflict(d, server);
				drafts[key] = rebased;
				queue = queue.map((x) => (x.key === key ? server : x));
				if (currentKey === key) void loadSimilar(server);
				const named = clashes.map((c) => `${FIELD_LABEL[c.field]} is now "${c.value}"`).join('; ');
				notice = { text: `${key} changed meanwhile${named ? `: ${named}` : ''}. Check and ${again} again.${tail}` };
			}
			void reload(); // a conflict has no live event of its own
		} catch (e) {
			notice = { text: `${key} changed meanwhile and could not be reloaded: ${errorMessage(e)}${tail}` };
		}
	}

	async function drop(): Promise<void> {
		const s = current;
		const d = draft;
		if (!s || !d || left || busy) return;
		const version = d.base.version;
		const dup = d.duplicateOf;
		if (!armed) {
			arm = { key: s.key, version };
			notice = { text: armText(s, dup) };
			return;
		}
		arm = null;
		const order = queue.map((x) => x.key);
		const tail = dup ? ' The duplicate comment is saved.' : '';
		busy = true;
		try {
			if (dup && !(await commentDuplicate(s.key, dup))) return;
			const out = await postMove({ key: s.key, version }, 'done', { resolution: 'dropped' });
			if (out.kind !== 'moved') {
				notice = { text: `${why(out)}${tail}` };
				return;
			}
			finished(s.key, order, { text: `Dropped ${s.key}${dup ? ` as a duplicate of ${dup}` : ''}.`, open: s.key }, dropUndo(out.story, !!dup));
		} catch (e) {
			if (e instanceof Conflict) await conflicted(s.key, d, 'drop', tail);
			else notice = { text: `${s.key} was not dropped: ${errorMessage(e)}${tail}` };
		} finally {
			busy = false;
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
				notice = { text: `Undone: ${back.key} is back in the queue.` };
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
				if (out.kind !== 'moved') throw new Error(why(out));
				returned(out.story);
				notice = { text: `Undone: ${dropped.key} is back in todo.${duplicate ? ' The duplicate comment stays.' : ''}` };
			}
		};
	}

	async function undo(): Promise<void> {
		const u = undoable;
		if (!u || busy) return;
		undoable = null; // one Undo per action
		busy = true;
		try {
			await u.run();
		} catch (e) {
			const text = e instanceof Conflict ? `${u.key} changed since; undo it in the drawer.` : `Undo failed: ${errorMessage(e)}`;
			notice = { text, open: u.key };
		} finally {
			busy = false;
			focusBack();
		}
	}

	onMount(() => void reload());
	onLive(reload);
</script>

<PageHeader title="Triage" />
<!-- The keys are this area's, not the page's: a keydown from anywhere else never reaches this handler. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="triage" bind:this={area} onkeydown={keydown}>
	<div class="notice">
		<p role="status" aria-live="polite" data-testid="triage-status">{notice?.text ?? ''}</p>
		{#if notice?.open}<a href={storyHref(page.url, notice.open)} data-sveltekit-reset="false">Open {notice.open}</a>{/if}
		{#if undoable}<button type="button" onclick={undo}>Undo {undoable.action} {undoable.key} <kbd aria-hidden="true">u</kbd></button>{/if}
	</div>
	{#if loadError}
		<div class="error" role="alert">
			<p>Couldn't load the queue: {loadError}</p>
			<button type="button" onclick={() => void reload()}>Retry</button>
		</div>
	{/if}
	{#if !loaded}
		{#if !loadError}<p class="loading">Loading…</p>{/if}
	{:else if !queue.length && !left && currentKey === null}
		<div class="empty">
			<h2 id="triage-empty" tabindex="-1" bind:this={emptyEl}>Nothing to triage</h2>
			<p><a href="/backlog">Go to the backlog</a></p>
		</div>
	{:else}
		<ul
			class="queue"
			role="listbox"
			aria-label="Untriaged"
			tabindex="0"
			bind:this={list}
			aria-activedescendant={currentKey && position >= 0 ? `triage-opt-${currentKey}` : undefined}
		>
			{#each queue as s (s.key)}
				<!-- svelte-ignore a11y_click_events_have_key_events -->
				<li role="option" id="triage-opt-{s.key}" data-key={s.key} aria-selected={s.key === currentKey} onclick={() => select(s.key)}>
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
					<dt>j / k</dt><dd>next / previous (stops at the ends)</dd><dt>↓ / ↑</dt><dd>the same, with focus on the queue</dd>
					<dt>e</dt><dd>epic (Enter on a new epic moves on to criteria)</dd><dt>t</dt><dd>kind</dd><dt>c</dt><dd>criteria</dd>
					<dt>Ctrl/Cmd+Enter</dt><dd>place</dd><dt>d, d</dt><dd>drop</dd>
					<dt>1–{SIMILAR_SHOWN}</dt><dd>mark the item as a duplicate of a similar one</dd><dt>s</dt><dd>skip (after the last, back to the first)</dd>
					<dt>o</dt><dd>open in the drawer</dd><dt>u</dt><dd>undo</dd><dt>Escape</dt><dd>leave a field, cancel a drop</dd>
					<dt>?</dt><dd>this list</dd>
				</dl>
			{/if}
			{#if current && draft}
				<TriageItem
					bind:this={pane}
					item={current}
					bind:draft={drafts[current.key]!}
					{epics}
					hits={similar}
					disabled={!!left || busy}
					gone={left ? whereNow(left) : null}
					onplace={place}
					{armed}
					ondrop={drop}
					onskip={skip}
					onmark={mark}
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
	.error { grid-column: 1 / -1; color: var(--danger); display: flex; flex-wrap: wrap; gap: 8px; align-items: baseline; }
	.error p { margin: 0; }
	.loading { grid-column: 1 / -1; color: var(--muted); }
	.queue { list-style: none; margin: 0; padding: 0; overflow-y: auto; max-height: calc(100vh - var(--top-bar-height, 0px) - 120px); }
	.queue li { padding: 6px 8px; cursor: pointer; border-radius: 6px; }
	/* The background marks the current item; the focus ring is the listbox's (inset, so the scroll box does not clip it). */
	.queue li[aria-selected='true'] { background: var(--selection); }
	.queue:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
	.state { color: var(--muted); }
	.pane { min-width: 0; }
	.counter button { margin-left: 8px; }
	@media (max-width: 719px) {
		.triage { grid-template-columns: 1fr; }
		.queue { display: none; }
	}
</style>
