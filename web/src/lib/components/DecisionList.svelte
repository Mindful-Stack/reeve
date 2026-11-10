<script lang="ts">
	import Glyph from '#lib/components/Glyph.svelte';
	import ActionsMenu, { type MenuItem } from '#lib/ui/ActionsMenu.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';
	import { levelOfItem } from '#lib/glyphs.ts';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import type { AffectedStory, ClosureKind, Comment, RequiredDecision, Reversibility } from '#lib/api/types.ts';
	import { fullLabel } from '#lib/board.ts';
	import { closeActions, dateLine, decisionStatus, pruneExpanded, recordMismatch, today } from '#lib/decisions.ts';
	import { age, errorMessage, short } from '#lib/format.ts';
	import { draftKey } from '#lib/idempotency.ts';
	import { decisionDrafts, draftFor } from '#lib/decisionDrafts.svelte.ts';
	import { tick as nextRender, untrack, type Snippet } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { everyVisible, minuteClock, onLive } from '#lib/events.svelte.ts';
	import { safePrUrl } from '#lib/prs.ts';
	import CommentBody from '#lib/components/CommentBody.svelte';
	import { followBase } from '#lib/editorBase.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { DECISION_SAME, decisionDraft, decisionEdit, fieldsText, mergeDraft, type DecisionDraft } from '#lib/forms.ts';

	// Each row is flat: what the decision is on, its status, record and decider, a Discussion toggle that expands the
	// thread (where ratification is discussed) and an Actions menu (Edit, Close) that opens its form under the row, so
	// every view that lists decisions also carries the thread. The UI only links records; it writes nothing into them
	// (kb/ADR-0004).
	type Group = { id: string; folded: boolean; listId?: string };

	let {
		decisions,
		itemHref,
		onchanged,
		poll = true,
		rest,
		showRest = false,
		restId,
		between,
		empty = 'No decisions.',
		blocks,
		groupOf,
		groups = [],
		heading
	}: {
		decisions: RequiredDecision[];
		/** The stories each decision keeps out of active work, by decision id (the queue's `blocks`). A decision missing
		 *  from the map shows no blocks line, so a view without the queue (the drawers, the item pages) passes none. */
		blocks?: ReadonlyMap<number, AffectedStory[]>;
		/** Where the item a decision blocks opens; when absent (on the item's own page) the item column is left out. */
		itemHref?: (key: string) => string;
		/** Reloads the list: after a decision changed (closed, or a conflict), and every two minutes while visible. */
		onchanged: () => Promise<void>;
		/** Whether this list runs the two-minute refetch; a page showing several lists lets one of them poll. */
		poll?: boolean;
		/** Splits the list in two: the decisions it picks are listed after `between`, and only while `showRest`. One list
		 *  over both groups keeps a row's editor, comment draft and expanded state when it moves between them. */
		rest?: (d: RequiredDecision) => boolean;
		showRest?: boolean;
		/** The id of the second group's list, for the fold's aria-controls. */
		restId?: string;
		/** Rendered between the groups (the fold control); with `rest` only. */
		between?: Snippet;
		/** Said when the first group is empty (with `groupOf`: when every unfolded group is). */
		empty?: string;
		/** Splits the list into named groups (the queue's): each decision is listed in the group this names. One list over
		 *  every group keeps a row's editor, comment draft and focus when it moves between them. `rest` is the two-group
		 *  case of the same split. */
		groupOf?: (d: RequiredDecision) => string;
		/** The groups in render order. A folded group shows only its heading; `listId` is its list's id when unfolded. */
		groups?: Group[];
		/** Rendered before each group that has a decision in it, folded or not, with the group and how many rows are placed
		 *  in it (a row pinned in its old group counts there). */
		heading?: Snippet<[string, number]>;
	} = $props();
	/** `rest` as two groups: the first always shown, the rest folded unless `showRest`. */
	const FIRST = 'first';
	const REST = 'rest';
	const layout = $derived<Group[]>(
		groupOf ? groups : [{ id: FIRST, folded: false }, ...(rest ? [{ id: REST, folded: !showRest }] : [])]
	);
	function groupFor(d: RequiredDecision): string {
		if (groupOf) return groupOf(d);
		return rest?.(d) ? REST : FIRST;
	}

	/** The group each decision was rendered in by the last render, for the shown groups only (not reactive: read by
	 *  `place`, written after render). */
	let lastShown = new Map<number, string>();
	/** A decision's group, unless that group is folded and the decision was last shown in a group still unfolded while it
	 *  is being edited or closed, or holds the remembered focus: then it stays put until the form closes or focus leaves,
	 *  so a status change cannot hide the field the user is working in. */
	function place(d: RequiredDecision, folded: ReadonlyMap<string, boolean>): string {
		const g = groupFor(d);
		if (!folded.get(g)) return g;
		const was = lastShown.get(d.id);
		// A closed decision has no form (its state is dropped on arrival), so only focus can pin it.
		const busyHere = (!d.closed && (!!edits[d.id] || !!closing[d.id])) || focusedId === d.id;
		return busyHere && was !== undefined && folded.get(was) === false ? was : g;
	}
	/** Each group of the layout with the decisions placed in it, in input order; a decision of an unknown group is left out. */
	const placed = $derived.by(() => {
		const folded = new Map(layout.map((g) => [g.id, g.folded]));
		const by = new Map<string, RequiredDecision[]>(layout.map((g) => [g.id, []]));
		for (const d of decisions) by.get(place(d, folded))?.push(d);
		return layout.map((g) => ({ ...g, rows: by.get(g.id)! }));
	});
	const first = $derived(placed.find((g) => g.id === FIRST)?.rows ?? []);
	const second = $derived(placed.find((g) => g.id === REST)?.rows ?? []);

	// After each render of the groups: a row that was re-created gets its focused field back.
	$effect(() => {
		const shown = new Map<number, string>();
		for (const g of placed) if (!g.folded) for (const d of g.rows) shown.set(d.id, g.id);
		lastShown = shown;
		untrack(restoreFocus);
	});

	let threads = $state<Record<number, Comment[]>>({});
	let reasons = $state<Record<number, string>>({});
	/** A row's thread, comment and close failures, shown at the top of its body. */
	let errors = $state<Record<number, string>>({});
	/** The editor's own failures, shown inside its form (an error above a long thread is out of sight). */
	let editErrors = $state<Record<number, string>>({});
	let busy = $state<number | null>(null);
	/** Which discussions are expanded now; a live refetch reloads only those, a collapsed one reloads when reopened. */
	const expanded = new SvelteSet<number>();

	/** A decision that changes group (blocking to accepted and back) moves to the other keyed loop, so its row is
	 *  re-created and the browser drops focus to <body>. The focused control is remembered by decision and its
	 *  `data-field` (a semantic name every focusable control in a row carries; never a position or label, which a
	 *  concurrent change could shift), and put back with its selection in the new row, once, in the render that moved it,
	 *  and only when nothing else holds focus by then. */
	const uid = $props.id();
	/** Wraps the lists, for finding the heading of the section the list sits in. */
	let root = $state<HTMLDivElement>();
	type Focused = { id: number; field: string; el: HTMLElement };
	let focused: Focused | null = null;
	/** The decision holding the remembered focus, reactive: such a row is pinned in the group it was shown in (`place`). */
	let focusedId = $state<number | null>(null);
	function setFocused(f: Focused | null) {
		focused = f;
		focusedId = f?.id ?? null;
	}

	function selection(el: Element): [number | null, number | null] {
		try {
			if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return [el.selectionStart, el.selectionEnd];
		} catch {
			// a date input has no selection
		}
		return [null, null];
	}

	function remember(id: number, e: FocusEvent) {
		const el = e.target as HTMLElement;
		// Focus inside the row's open Actions menu counts as staying on its trigger (Escape or a choice hands it back),
		// so opening the menu keeps the memory and the row's pin.
		if (el.closest('[role="menu"]')) return;
		const field = el.dataset.field;
		// A control without a data-field (a link inside a rendered comment) is not restored.
		setFocused(field ? { id, field, el } : null);
	}

	/** Forgets the field once focus really left it (it is still in the page); a removed row keeps it for restoreFocus. */
	function forget(e: FocusEvent) {
		const f = focused;
		// Leaving the trigger's open menu (Tab) is leaving the trigger.
		const fromMenu = !!(e.target as Element).closest?.('[role="menu"]');
		if (!f || (e.target !== f.el && !fromMenu)) return;
		setTimeout(() => {
			const active = document.activeElement;
			if (active?.closest('[role="menu"]')) return; // into the trigger's own menu: still there
			if (focused === f && f.el.isConnected && active !== f.el) setFocused(null);
		});
	}

	function restoreFocus() {
		const f = focused;
		if (!f || f.el.isConnected) return;
		setFocused(null); // one attempt, in the render that moved the row: a stale memory must never pull focus later
		const active = document.activeElement;
		if (active && active !== document.body) return; // never take focus from another control
		const el = document.querySelector<HTMLElement>(`[data-decision-row="${uid}-${f.id}"] [data-field="${f.field}"]`);
		if (!el) return;
		// A detached field keeps its selection: read it now, not when focus arrived.
		const [start, end] = selection(f.el);
		el.focus();
		if (start !== null && (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) {
			try {
				el.setSelectionRange(start, end ?? start);
			} catch {
				// not a text field
			}
		}
		setFocused({ id: f.id, field: f.field, el });
	}
	/** Per-thread request generation: a refetch overlapping a newer one (or a post's answer) is dropped. */
	const generation: Record<number, number> = {};

	/** The viewer's date, from the minute clock, so the date line's "open N days" moves on an idle page. */
	const now = $derived(today(new Date(minuteClock.now)));

	/** A scheduled check that finds what the last one found moves only `checked_at` and logs nothing, so no live event
	 *  reloads the list: it reloads itself every two minutes while visible, as long as an open decision has a record to
	 *  check. Two minutes keeps the label of a healthy decision at 15 minutes or less (runs start at most twelve
	 *  minutes apart), which deploy/README.md takes as the sign that runs are failing. */
	const REFETCH_MS = 2 * 60_000;
	everyVisible(() => {
		if (poll && decisions.some((d) => d.record && !d.closed)) void onchanged();
	}, REFETCH_MS);

	/** The PR link of a proposed record, only when http(s): plain text otherwise, so a stored `javascript:` url never runs. */
	function prHref(d: RequiredDecision): string | null {
		return d.check.kind === 'proposed_pr_open' && d.pr_url ? safePrUrl(d.pr_url) : null;
	}

	/** One idempotency key per comment being written; `draftKey` keeps it across retries of the same text only. */
	function write(id: number, body: string) {
		draftFor(id).body = body;
	}

	/** Writes the failure's text into `into` (the row's errors unless the editor's); a conflict also reloads the list. */
	async function failed(id: number, e: unknown, into: Record<number, string> = errors) {
		if (e instanceof Refused) into[id] = e.reasons.map((r) => fullLabel(r)).join('; ');
		else if (e instanceof Conflict) {
			into[id] = 'Someone else changed this decision; the list was reloaded.';
			await onchanged();
		} else if (e instanceof ApiError) into[id] = e.message;
		else into[id] = errorMessage(e);
	}

	/** Loads (or reloads) one discussion. Drafts live in `decisionDrafts`, which this never touches, so an unsent comment and its
	 *  idempotency key survive every refetch. */
	async function loadThread(id: number) {
		const seq = (generation[id] = (generation[id] ?? 0) + 1);
		try {
			const thread = await unwrap(api.GET('/api/v1/decisions/{id}/comments', { params: { path: { id } } }));
			if (seq !== generation[id]) return;
			threads[id] = thread;
		} catch (e) {
			if (seq !== generation[id]) return;
			await failed(id, e);
		}
	}

	function toggled(id: number, open: boolean) {
		if (!open) {
			expanded.delete(id);
			return;
		}
		expanded.add(id);
		// Every reopen refetches: a collapsed thread is not refreshed live, so it may be behind.
		void loadThread(id);
	}

	// A comment from another browser or the CLI is an activity entry: refresh every expanded discussion.
	onLive(() => {
		// Prune ids whose row left the list (e.g. a decision closed while closed ones are hidden): no refetch for them.
		pruneExpanded(expanded, decisions);
		for (const id of expanded) void loadThread(id);
	});

	async function post(e: SubmitEvent, id: number) {
		e.preventDefault();
		const d = decisionDrafts[id];
		const body = d?.body.trim();
		if (!d || !body || busy !== null) return;
		busy = id;
		delete errors[id];
		try {
			const thread = await unwrap(
				api.POST('/api/v1/decisions/{id}/comments', {
					params: { path: { id } },
					body: { body, idempotency_key: draftKey(d.id, body) }
				})
			);
			// The post's answer is newer than any refetch still in flight.
			generation[id] = (generation[id] ?? 0) + 1;
			threads[id] = thread;
			// Only if the composer still holds what was sent (it is disabled while posting, so this is a guard, not a race).
			if (decisionDrafts[id]?.body.trim() === body) delete decisionDrafts[id];
		} catch (err) {
			await failed(id, err);
		} finally {
			busy = null;
		}
	}

	/** An open editor: the draft, and the values and version it started from (its base). `inline` is a change sent
	 *  straight from the row (the Classify picker, "Use the record's value"): its form stays hidden unless the send
	 *  fails, and then shows the merged draft and the error like any edit. */
	type Editing = { draft: DecisionDraft; from: DecisionDraft; version: number; inline?: boolean };
	let edits = $state<Record<number, Editing>>({});
	/** Each row's open editor, for focusing its first field. */
	const editForms: Record<number, HTMLFormElement | null> = {};
	/** The rows whose close form (the reason and Decide or Withdraw) is open, and which, chosen from the Actions menu. */
	let closing = $state<Record<number, ClosureKind>>({});
	/** What each close kind is called in the menu and on its submit button, and what its reason field asks for, with an
	 *  example where it helps (kb/ADR-0013 R3). */
	const CLOSE_TEXT: Record<ClosureKind, { action: string; label: string; placeholder?: string }> = {
		decided: { action: 'Decide', label: "What was decided, and where it's written down", placeholder: 'docs/behaviour/orgs.md org.R7' },
		withdrawn: { action: 'Withdraw', label: "Why it's no longer needed" }
	};
	const closeForms: Record<number, HTMLFormElement | null> = {};
	/** The chip's words for a classification (kb/ADR-0013 R1). */
	const REVERSIBILITY_TEXT: Record<Reversibility, string> = { one_way: 'One-way', two_way: 'Two-way' };
	/** The rows whose "Blocks N stories" disclosure is open. */
	const blocksOpen = new SvelteSet<number>();

	function blockedOf(d: RequiredDecision): AffectedStory[] {
		return blocks?.get(d.id) ?? [];
	}

	function stories(n: number): string {
		return `${n} ${n === 1 ? 'story' : 'stories'}`;
	}

	function classifyItems(d: RequiredDecision): MenuItem[] {
		return (['one_way', 'two_way'] as const).map((r) => ({
			label: REVERSIBILITY_TEXT[r],
			onselect: () => classify(d, r),
			disabled: !canClassify(d)
		}));
	}

	/** A row change to the classification waits while a save runs, an editor is open or a close form is (a Decide chosen
	 *  on a two-way decision must not turn one-way under it). */
	function canClassify(d: RequiredDecision): boolean {
		return busy === null && !edits[d.id] && !closing[d.id] && !d.closed;
	}

	/** Sets the classification from the row: the editor's own path (only the changed field is sent, under the row's
	 *  version), so a 409 merges per field and opens the editor on the merge. */
	function classify(d: RequiredDecision, r: Reversibility) {
		if (!canClassify(d)) return;
		delete editErrors[d.id];
		const from = decisionDraft(d);
		edits[d.id] = { draft: { ...from, reversibility: r }, from, version: d.version, inline: true };
		void submitEdit(d);
	}

	/** The row's Actions menu trigger, where focus goes back when one of its forms closes. */
	function actionsOf(id: number): HTMLElement | null {
		return document.querySelector<HTMLElement>(`[data-decision-row="${uid}-${id}"] [data-field="actions"]`);
	}

	function menu(d: RequiredDecision): MenuItem[] {
		return [
			{ label: 'Edit', onselect: () => startEdit(d), disabled: !!edits[d.id] },
			...closeActions(d.reversibility).map((kind) => ({ label: CLOSE_TEXT[kind].action, onselect: () => startClose(d.id, kind), disabled: !!closing[d.id] }))
		];
	}

	function startClose(id: number, kind: ClosureKind) {
		closing[id] = kind;
		void nextRender().then(() => closeForms[id]?.querySelector('input')?.focus());
	}

	/** The ids of the rows on screen, in order. */
	function rowIds(): number[] {
		const prefix = `${uid}-`;
		return [...document.querySelectorAll<HTMLElement>(`[data-decision-row^="${prefix}"]`)].map((el) =>
			Number(el.dataset.decisionRow!.slice(prefix.length))
		);
	}

	/** After a close and its reload: unless focus already landed (restoreFocus, or the user moved it), it goes to the
	 *  closed row's Discussion toggle; if the row left the list, to the next row's Actions, else the previous row's, else
	 *  the heading of the section holding the list. Never left on <body>. */
	function afterClose(id: number, form: HTMLFormElement | null | undefined, before: number[]) {
		const active = document.activeElement;
		if (active && active !== document.body && !form?.contains(active)) return;
		if (focused?.el === form) setFocused(null);
		const row = (rid: number) => document.querySelector<HTMLElement>(`[data-decision-row="${uid}-${rid}"]`);
		const target = (rid: number) =>
			row(rid)?.querySelector<HTMLElement>('[data-field="actions"]') ?? row(rid)?.querySelector<HTMLElement>('[data-field="discussion"]');
		let el = row(id)?.querySelector<HTMLElement>('[data-field="discussion"]') ?? null;
		const at = before.indexOf(id);
		for (const rid of before.slice(at + 1)) el ??= target(rid) ?? null;
		for (const rid of before.slice(0, Math.max(0, at)).reverse()) el ??= target(rid) ?? null;
		if (!el) {
			// The heading of the section holding this list ("Required decisions" in the drawer), else the page's.
			el = (root?.closest('section, aside, main') ?? document).querySelector<HTMLElement>('h1, h2, h3');
			el?.setAttribute('tabindex', '-1');
		}
		el?.focus();
	}

	/** Cancel on the close form: like closeEdit, focus goes back to the row's Actions menu if it was in the form. */
	function cancelClose(id: number) {
		const form = closeForms[id];
		const free = focusIsFree(form);
		if (free && form) setFocused({ id, field: 'actions', el: form });
		delete closing[id];
		if (free) void nextRender().then(() => actionsOf(id)?.focus());
	}

	/** Opens the editor focused on its first field, or on the due date (the date line's "set date"). */
	function startEdit(d: RequiredDecision, at: 'title' | 'due' = 'title') {
		if (!edits[d.id]) {
			delete editErrors[d.id];
			edits[d.id] = { draft: decisionDraft(d), from: decisionDraft(d), version: d.version };
		}
		void nextRender().then(() => editForms[d.id]?.querySelector<HTMLElement>(`[data-field="${at}"]`)?.focus());
	}

	/** Cancel, an unchanged save and a successful one all close the editor and return focus to the row's Actions menu. */
	function closeEdit(id: number) {
		// Only if focus is still in the editor or on the page: a slow save must not take it from where the user went.
		const form = editForms[id];
		const free = focusIsFree(form);
		// Hand the remembered focus to the row's Actions menu before the editor goes: the row stays pinned (focusedId) and
		// restoreFocus, in the render that removes the form, focuses the menu there. The pin then lasts until focus leaves.
		if (free && form) setFocused({ id, field: 'actions', el: form });
		delete edits[id];
		delete editErrors[id];
		if (free) void nextRender().then(() => actionsOf(id)?.focus());
	}

	/** A failed save leaves the editor open with its alert: focus goes back to its first field (its fields were disabled
	 *  while saving), unless the user moved it elsewhere. */
	function refocusEdit(id: number) {
		void nextRender().then(() => {
			if (focusIsFree(editForms[id])) editForms[id]?.querySelector('input')?.focus();
		});
	}

	// A decision that left the list (reassigned, or closed while closed ones are hidden) takes its draft and error with it.
	$effect(() => {
		const ids = new Set(decisions.map((d) => d.id));
		untrack(() => {
			for (const id of Object.keys(edits).map(Number)) if (!ids.has(id)) delete edits[id];
			for (const id of Object.keys(errors).map(Number)) if (!ids.has(id)) delete errors[id];
			for (const id of Object.keys(editErrors).map(Number)) if (!ids.has(id)) delete editErrors[id];
			for (const id of Object.keys(closing).map(Number)) if (!ids.has(id)) delete closing[id];
			for (const id of blocksOpen) if (!ids.has(id)) blocksOpen.delete(id);
		});
	});

	/** Said on a row whose open form went because someone else closed the decision. */
	const DROPPED = 'Someone else closed this decision; your unsent changes were dropped.';

	// A decision a colleague closed under its open editor or close form: the form goes (a closed row has none), so its
	// state goes too, or it would pin the closed row in its old group. Before the render, while the form is still in the
	// page: focus in it is handed to the row's Discussion toggle (restoreFocus, in the render that removes it).
	$effect.pre(() => {
		const gone = decisions.filter((d) => d.closed);
		untrack(() => {
			for (const d of gone) {
				const ed = edits[d.id];
				const kind = closing[d.id];
				// A save or close being sent settles its own form and says what happened (submitEdit, close).
				if ((!ed && !kind) || busy === d.id) continue;
				const holder = [editForms[d.id], closeForms[d.id]].find((f) => f?.contains(document.activeElement));
				if (holder) setFocused({ id: d.id, field: 'discussion', el: holder });
				delete edits[d.id];
				delete editErrors[d.id];
				delete closing[d.id];
				delete reasons[d.id];
				// An inline change waiting for nothing was never a form the user saw.
				if ((ed && !ed.inline) || kind) errors[d.id] = DROPPED;
			}
		});
	});

	function setEdit<K extends keyof DecisionDraft>(id: number, field: K, value: DecisionDraft[K]) {
		const ed = edits[id];
		if (ed) ed.draft[field] = value;
	}

	function saveEdit(e: SubmitEvent, d: RequiredDecision) {
		e.preventDefault();
		void submitEdit(d);
	}

	/** An inline change that could not be sent shows its editor, so the merged draft and the error are in sight. */
	function showEditor(id: number) {
		const ed = edits[id];
		if (ed?.inline) ed.inline = false;
	}

	async function submitEdit(d: RequiredDecision) {
		const ed = edits[d.id];
		if (!ed || busy !== null) return;
		const out = decisionEdit(ed.from, ed.draft);
		if (out.kind === 'unchanged') {
			closeEdit(d.id);
			return;
		}
		if (out.kind === 'invalid') {
			editErrors[d.id] = out.error;
			showEditor(d.id);
			return;
		}
		// An inline change starts from the row's control (the chip, the mismatch button), which the reload removes: focus
		// then moves to the row's Actions menu, the way a closing editor hands it back.
		const trigger = ed.inline ? document.activeElement : null;
		// The list may have been refetched since the editor opened: the base advances only if nobody changed these fields.
		const { version } = followBase(
			{ version: ed.version, text: fieldsText(ed.from) },
			{ version: d.version, text: fieldsText(decisionDraft(d)) }
		);
		busy = d.id;
		delete editErrors[d.id];
		try {
			await unwrap(api.PATCH('/api/v1/decisions/{id}', { params: { path: { id: d.id } }, body: { version, ...out.body } }));
			// Only while focus is still on that control: focus the user moved elsewhere (another row) keeps its memory and pin.
			if (trigger instanceof HTMLElement && document.activeElement === trigger && focused?.el === trigger)
				setFocused({ id: d.id, field: 'actions', el: trigger });
			closeEdit(d.id);
			await onchanged();
		} catch (err) {
			await failed(d.id, err, editErrors); // a Conflict reloads the list and says so
			const reloaded = decisions.find((x) => x.id === d.id);
			const cur = edits[d.id];
			if (err instanceof Conflict && reloaded?.closed) {
				// The editor goes with the open row, so its error would go unseen: say so where the closed row shows errors.
				errors[d.id] = 'Someone else closed this decision; your edit was not saved.';
				delete edits[d.id];
				delete editErrors[d.id];
			} else if (err instanceof Conflict && reloaded && cur) {
				// Only a conflict rebases: the user's changed fields stay, every other field takes the reloaded value.
				const server = decisionDraft(reloaded);
				edits[d.id] = {
					draft: mergeDraft(DECISION_SAME, cur.from, cur.draft, server),
					from: server,
					version: reloaded.version
				};
			}
			showEditor(d.id);
			if (edits[d.id]) refocusEdit(d.id);
		} finally {
			busy = null;
		}
	}

	async function close(e: SubmitEvent, id: number) {
		e.preventDefault();
		const reason = (reasons[id] ?? '').trim();
		const kind = closing[id];
		if (!reason || !kind || busy !== null) return;
		busy = id;
		delete errors[id];
		try {
			await unwrap(api.POST('/api/v1/decisions/{id}/close', { params: { path: { id } }, body: { kind, reason } }));
			reasons[id] = '';
			const form = closeForms[id];
			const free = focusIsFree(form);
			// The rows on screen before the reload, for a neighbour to take focus if the closed row leaves the list.
			const before = rowIds();
			// The form goes: the memory moves to the row's Discussion toggle, which restoreFocus focuses in the closed row.
			if (free && form) setFocused({ id, field: 'discussion', el: form });
			delete closing[id];
			await onchanged();
			// Always, not only when focus was free before the reload: a Cancel pressed while the close was pending put focus
			// on the row's Actions, which the reload may remove. afterClose leaves focus that still sits on a live control.
			void nextRender().then(() => afterClose(id, form, before));
		} catch (err) {
			await failed(id, err);
		} finally {
			busy = null;
		}
	}
</script>

{#snippet classifyTrigger()}Classify{/snippet}

{#snippet story(s: AffectedStory)}<span class="key">{s.key}</span> {s.title}{/snippet}

{#snippet chip(d: RequiredDecision)}
	{#if d.reversibility}
		<span class="chip">{REVERSIBILITY_TEXT[d.reversibility]}</span>
	{:else if !d.closed}
		<ActionsMenu label="Classify {d.title}" field="classify" class="chip classify" items={classifyItems(d)} trigger={classifyTrigger} />
	{/if}
{/snippet}

{#snippet row(d: RequiredDecision)}
	{@const tone = d.closed ? 'muted' : d.check.kind === 'accepted' ? 'ready' : 'warn'}
	{@const when = dateLine(d, now)}
	{@const blocked = blockedOf(d)}
	{@const mismatch = recordMismatch(d)}
	<li class:closed={!!d.closed} data-decision-row="{uid}-{d.id}" onfocusin={(e) => remember(d.id, e)} onfocusout={forget}>
		<!-- One line of cells in a fixed order, each present even when empty, so the list's columns line up. -->
		<div class="row">
			<span class="cell headline" data-col="title"><span class="title">{d.title}</span>{@render chip(d)}</span>
			<span class="cell decider" data-col="decider" title={d.closed ? undefined : d.decider}>{#if !d.closed}{short(d.decider)}<span class="visually-hidden">{' decides'}</span>{/if}</span>
			<span class="cell when" data-col="when"
				>{#if !d.closed}<span class={when.tone}>{when.text}</span>
					{#if !d.due}
						<button
							type="button"
							class="ghost inline"
							data-field="set-due"
							aria-label="Set the due date of {d.title}"
							disabled={busy !== null || !!edits[d.id]?.inline}
							onclick={() => startEdit(d, 'due')}>set date</button
						>
					{/if}{/if}</span
			>
			<span class="cell blocks-line" data-col="blocks"
				>{#if blocked.length}<button
						type="button"
						class="ghost inline"
						data-field="blocks"
						aria-label="Blocks {stories(blocked.length)}: {d.title}"
						aria-expanded={blocksOpen.has(d.id)}
						aria-controls={blocksOpen.has(d.id) ? `${uid}-${d.id}-blocked` : undefined}
						onclick={() => (blocksOpen.has(d.id) ? blocksOpen.delete(d.id) : blocksOpen.add(d.id))}
						>Blocks {stories(blocked.length)} <span aria-hidden="true">{blocksOpen.has(d.id) ? '▾' : '▸'}</span></button
					>{/if}</span
			>
			<span class="cell on" data-col="on"
				>{#if itemHref}on {d.on_type} <a class="key" data-field="item" href={itemHref(d.on)}><Glyph level={levelOfItem(d.on_type)} />{d.on}</a>{/if}</span
			>
			<span class="cell check" data-col="check">
				{#if prHref(d)}
					<a href={prHref(d)} data-field="pr" target="_blank" rel="noopener noreferrer"><StatusDot {tone} label={decisionStatus(d)} /></a>
				{:else}
					<StatusDot {tone} label={decisionStatus(d)} />
				{/if}
			</span>
			<button
				type="button"
				class="ghost discussion"
				data-col="discussion"
				data-field="discussion"
				aria-expanded={expanded.has(d.id)}
				aria-controls={expanded.has(d.id) ? `${uid}-${d.id}-discussion` : undefined}
				aria-label="Discussion of {d.title}"
				onclick={() => toggled(d.id, !expanded.has(d.id))}>Discussion</button
			>
			<span class="cell actions" data-col="actions"
				>{#if !d.closed}<ActionsMenu label="Actions for {d.title} on {d.on}" field="actions" items={menu(d)} />{/if}</span
			>
		</div>

		{#if d.closed || errors[d.id] || (edits[d.id] && !edits[d.id].inline) || closing[d.id] || expanded.has(d.id) || (blocked.length && blocksOpen.has(d.id)) || mismatch}
			<div class="body">
				{#if blocked.length && blocksOpen.has(d.id)}
					<ul class="blocked" id="{uid}-{d.id}-blocked">
						{#each blocked as s (s.key)}
							<li>
								{#if itemHref}
									<a data-field="blocked-{s.key}" href={itemHref(s.key)}>{@render story(s)}</a>
								{:else}
									{@render story(s)}
								{/if}
							</li>
						{/each}
					</ul>
				{/if}
				{#if d.closed}
					{@const verb = d.closed.kind === 'decided' ? 'Decided' : d.closed.kind === 'withdrawn' ? 'Withdrawn' : 'Closed'}
					<p class="outcome">
						<strong>{verb} by <span title={d.closed.by}>{short(d.closed.by)}</span>: {d.closed.reason}</strong>
						<span class="muted">
							{#if d.closed.via === 'automation'}via an automation token,{/if}
							{new Date(d.closed.at).toLocaleString()}
						</span>
					</p>
				{/if}
				{#if mismatch}
					<p class="mismatch-line">
						<span class="mismatch"><span aria-hidden="true">⚠</span> record says {REVERSIBILITY_TEXT[mismatch].toLowerCase()}</span>
						{#if !d.closed}
							<button
								type="button"
								class="ghost inline"
								data-field="use-record"
								aria-label="Use the record's value for {d.title}"
								disabled={!canClassify(d)}
								onclick={() => classify(d, mismatch)}>Use the record's value</button
							>
						{/if}
					</p>
				{/if}
				{#if errors[d.id]}<p class="error" role="alert">{errors[d.id]}</p>{/if}

				{#if !d.closed && edits[d.id] && !edits[d.id].inline}
					{@const ed = edits[d.id]}
					<form class="edit" bind:this={editForms[d.id]} aria-label="Edit decision {d.title}" onsubmit={(e) => saveEdit(e, d)}>
						<label>
							Title
							<input name="title" data-field="title" value={ed.draft.title} disabled={busy === d.id} oninput={(e) => setEdit(d.id, 'title', e.currentTarget.value)} />
						</label>
						<label>
							Decider
							<input name="decider" data-field="decider" required value={ed.draft.decider} disabled={busy === d.id} oninput={(e) => setEdit(d.id, 'decider', e.currentTarget.value)} />
						</label>
						<label>
							Due
							<input name="due" data-field="due" type="date" value={ed.draft.due} disabled={busy === d.id} oninput={(e) => setEdit(d.id, 'due', e.currentTarget.value)} />
						</label>
						<button type="button" data-field="clear-due" disabled={busy === d.id || !ed.draft.due} onclick={() => setEdit(d.id, 'due', '')}>Clear due date</button>
						<label>
							Reversibility
							<select
								name="reversibility"
								data-field="reversibility"
								value={ed.draft.reversibility}
								disabled={busy === d.id || !!closing[d.id]}
								onchange={(e) => setEdit(d.id, 'reversibility', e.currentTarget.value as Reversibility | '')}
							>
								<option value="">Unset</option>
								<option value="one_way">One-way</option>
								<option value="two_way">Two-way</option>
							</select>
						</label>
						{#if editErrors[d.id]}<p class="error" role="alert">{editErrors[d.id]}</p>{/if}
						<button type="button" data-field="cancel" onclick={() => closeEdit(d.id)}>Cancel</button>
						<button type="submit" data-field="save" disabled={busy !== null}>Save</button>
					</form>
				{/if}

				{#if !d.closed && closing[d.id]}
					{@const text = CLOSE_TEXT[closing[d.id]]}
					<form class="close" bind:this={closeForms[d.id]} aria-label="{text.action} decision {d.title}" onsubmit={(e) => close(e, d.id)}>
						<label>
							{text.label}
							<input
								name="reason"
								data-field="reason"
								required
								placeholder={text.placeholder}
								value={reasons[d.id] ?? ''}
								oninput={(e) => (reasons[d.id] = e.currentTarget.value)}
							/>
						</label>
						{#if d.reversibility === 'one_way'}
							<p class="muted hint">A one-way decision is decided by its accepted record; withdraw it only if it is no longer needed.</p>
						{/if}
						{#if closing[d.id] === 'withdrawn' && blocked.length}
							<div class="hint unblocks">
								<p>Withdrawing lifts the gate on {stories(blocked.length)}:</p>
								<ul>
									{#each blocked as s (s.key)}<li>{@render story(s)}</li>{/each}
								</ul>
							</div>
						{/if}
						<button type="submit" data-field="close" disabled={busy !== null || !(reasons[d.id] ?? '').trim()}>{text.action}</button>
						<button type="button" data-field="close-cancel" onclick={() => cancelClose(d.id)}>Cancel</button>
					</form>
				{/if}

				{#if expanded.has(d.id)}
					<section class="discussion-panel" id="{uid}-{d.id}-discussion" aria-label="Discussion of {d.title}">
						{#if d.record}
							<p class="record-line">
								<span class="record" title={d.record.record_id}>{d.record.repo}/{d.record.path}</span>
								{#if d.checked_at}
									· <time class="checked" datetime={d.checked_at} title={new Date(d.checked_at).toLocaleString()}>checked {age(d.checked_at, minuteClock.now)} ago</time>
								{/if}
							</p>
						{/if}
						{#if !threads[d.id]}
							<p class="muted">Loading…</p>
						{:else}
							<ol class="thread">
								{#each threads[d.id] as c (c.id)}
									<li>
										<span class="muted">{short(c.author)} · {age(c.at, minuteClock.now)}</span>
										<CommentBody markdown={c.body} />
									</li>
								{:else}
									<li class="muted">No comments yet.</li>
								{/each}
							</ol>
						{/if}
						<form onsubmit={(e) => post(e, d.id)}>
							<textarea
								rows="2"
								name="comment"
								data-field="comment"
								placeholder="Add to the discussion"
								aria-label="comment on {d.title}"
								value={decisionDrafts[d.id]?.body ?? ''}
								disabled={busy === d.id}
								oninput={(e) => write(d.id, e.currentTarget.value)}
							></textarea>
							<button type="submit" data-field="comment-submit" disabled={busy !== null || !decisionDrafts[d.id]?.body.trim()}>Comment</button>
						</form>
					</section>
				{/if}
			</div>
		{/if}
	</li>
{/snippet}

<div class="decision-list" bind:this={root}>
{#if groupOf}
	{#if placed.every((g) => g.folded || g.rows.length === 0)}
		<p class="empty">{empty}</p>
	{/if}
	{#each placed as g (g.id)}
		{#if g.rows.length}
			{@render heading?.(g.id, g.rows.length)}
			{#if !g.folded}
				<ul class="decisions" id={g.listId}>
					{#each g.rows as d (d.id)}{@render row(d)}{/each}
				</ul>
			{/if}
		{/if}
	{/each}
{:else if first.length === 0}
	<p class="empty">{empty}</p>
{:else}
	<ul class="decisions">
		{#each first as d (d.id)}{@render row(d)}{/each}
	</ul>
{/if}
{#if !groupOf}{@render between?.()}{/if}
{#if !groupOf && rest && showRest}
	<ul class="decisions" id={restId}>
		{#each second as d (d.id)}{@render row(d)}{/each}
	</ul>
{/if}
</div>

<style>
	/* One grid under every group, its lists and the parent's headings and folds: each row's cells sit on the same columns
	   (subgrid), so they line up down the whole list like a table's. The 10px row gap is the spacing the parents' grid
	   sections gave these children before the list had a box of its own; a parent that spaces them with margins instead
	   sets `--decision-list-row-gap="0"`. */
	.decision-list {
		container-type: inline-size;
		display: grid;
		grid-template-columns: minmax(16rem, 1fr) repeat(7, auto);
		gap: var(--decision-list-row-gap, 10px) 12px;
		min-width: 0;
	}
	.decision-list > :global(*) {
		grid-column: 1 / -1;
	}
	.decisions {
		display: grid;
		grid-template-columns: subgrid;
		row-gap: 0.4rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.decisions > li {
		display: grid;
		grid-column: 1 / -1;
		grid-template-columns: subgrid;
		align-items: center;
		padding: 6px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.closed .title {
		color: var(--muted);
	}
	.row {
		display: contents;
	}
	/* A cell never shrinks below its content (a squeezed cell would draw over its neighbour): a long date line or status
	   wraps inside its column instead, and a long decider is cut, its address on hover. */
	.cell {
		font-size: 14px;
		color: var(--muted);
	}
	.cell.headline {
		font-size: inherit;
		color: inherit;
	}
	.cell.decider {
		max-width: 10rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.cell .key {
		white-space: nowrap;
	}
	/* Too narrow for every cell on one line (the epic and initiative pages, the story drawer, a laptop-width window): the
	   title takes its own line beside the Actions menu and the rest wraps under it. */
	@container (max-width: 72rem) {
		.decisions > li {
			display: flex;
			flex-wrap: wrap;
			gap: 2px 12px;
		}
		.row > * {
			order: 1;
		}
		.cell.headline {
			flex: 1 1 calc(100% - 3rem);
			order: 0;
		}
		.cell.actions {
			order: 0;
		}
		.body {
			flex-basis: 100%;
			order: 2;
		}
		.cell:empty {
			display: none;
		}
		.discussion {
			margin-left: auto;
		}
	}
	.record {
		font-family: var(--mono);
	}
	.title {
		font-weight: 600;
	}
	.check a {
		text-decoration: none;
	}
	.check a:hover :global(.status-dot) {
		text-decoration: underline;
	}
	.headline {
		display: flex;
		flex-wrap: wrap;
		gap: 0.25rem 0.5rem;
		align-items: center;
	}
	.chip,
	.headline :global(.chip.classify) {
		padding: 0 0.45rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		font-size: 12px;
		line-height: 1.6;
		color: var(--muted);
	}
	.headline :global(.chip.classify) {
		border-color: var(--warn);
		background: var(--warn-soft);
		color: var(--foreground);
	}
	.when .warning,
	.mismatch {
		color: var(--warn);
	}
	.when .danger {
		color: var(--danger);
		font-weight: 600;
	}
	.inline {
		padding: 0 0.25rem;
		font-size: 14px;
	}
	.blocked,
	.unblocks ul {
		display: grid;
		gap: 2px;
		margin: 0;
		padding: 0 0 0 1rem;
		font-size: 14px;
	}
	.unblocks p {
		margin: 0;
	}
	.outcome {
		margin: 0;
	}
	.body {
		display: grid;
		grid-column: 1 / -1;
		gap: 0.5rem;
		padding: 4px 0;
	}
	.mismatch-line,
	.record-line {
		margin: 0;
		font-size: 14px;
		color: var(--muted);
	}
	.discussion-panel {
		display: grid;
		gap: 0.5rem;
	}
	.thread {
		display: grid;
		gap: 0.4rem;
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: 14px;
	}
	form {
		display: flex;
		gap: 0.5rem;
		align-items: flex-start;
	}
	.edit {
		flex-wrap: wrap;
	}
	.edit .error {
		flex-basis: 100%;
	}
	.close {
		flex-wrap: wrap;
	}
	.close label {
		display: flex;
		flex: 1;
		gap: 0.5rem;
		align-items: center;
	}
	.close .hint {
		flex-basis: 100%;
		order: 1;
	}
	textarea,
	input {
		flex: 1;
		font: inherit;
	}
	.empty,
	.muted {
		margin: 0;
		color: var(--muted);
	}
	.error {
		margin: 0;
		color: var(--danger);
	}
</style>
