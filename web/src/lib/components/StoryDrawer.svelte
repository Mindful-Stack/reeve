<script lang="ts">
	import Glyph from '#lib/components/Glyph.svelte';
	import { levelOfKind } from '#lib/glyphs.ts';
	import { onMount, tick } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import type {
		ActivityEntry,
		ItemRef,
		Comment,
		EditItem,
		Epic,
		Initiative,
		Readiness,
		Reason,
		RequiredDecision,
		Sprints,
		Story
	} from '#lib/api/types.ts';
	import { COLUMN_LABEL, COLUMNS, fullLabel, resolutionsFor, type Column, type Resolution } from '#lib/board.ts';
	import { filterDecisions } from '#lib/decisions.ts';
	import { onLive } from '#lib/events.svelte.ts';
	import { draftKey, newDraftKey } from '#lib/idempotency.ts';
	import { followBase, rebase, type EditorBase } from '#lib/editorBase.ts';
	import { session } from '#lib/me.svelte.ts';
	import { reportContext } from '#lib/nav.ts';
	import { openReport, reportLabel } from '#lib/report.svelte.ts';
	import { moveStep, postMove } from '#lib/moves.ts';
	import { PR_LABEL, safePrUrl } from '#lib/prs.ts';
	import { age, errorMessage, short } from '#lib/format.ts';
	import { storyHref } from '#lib/drawer.ts';
	import { expiry } from '#lib/urgency.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import ActivityList from '#lib/components/ActivityList.svelte';
	import Brief from '#lib/components/Brief.svelte';
	import Attachments from '#lib/components/Attachments.svelte';
	import ImageUploadsRow from '#lib/components/ImageUploadsRow.svelte';
	import { ImageUploads, attachable } from '#lib/attach.svelte.ts';
	import Comments from '#lib/components/Comments.svelte';
	import CriteriaEditor from '#lib/components/CriteriaEditor.svelte';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import MarkUrgentDialog from '#lib/components/MarkUrgentDialog.svelte';
	import UrgentLine from '#lib/components/UrgentLine.svelte';
	import OverridePrompt from '#lib/components/OverridePrompt.svelte';
	import ActionsMenu, { type MenuItem } from '#lib/ui/ActionsMenu.svelte';
	import Avatar from '#lib/ui/Avatar.svelte';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';

	// One story, over whatever view is open: the layout mounts this while `?story=KEY` is in the URL (keyed by KEY, so a new
	// key mounts a fresh drawer). Nothing changes until a reload: every write sends the story's current version and reloads.
	let { storyKey, onclose }: { storyKey: string; onclose: () => void } = $props();

	type PendingMove = { to: Column; resolution?: Resolution };
	/** A part of the drawer that reads as a value until its pen opens an editor. */
	type Field = 'title' | 'criteria' | 'decision' | 'owner' | 'labels' | 'epic' | 'goal' | 'blocker';
	/** A field, or the description (whose editor keeps its own state for the conflict base). Each has a pen or add button
	 *  `drawer-{name}-edit` and, while open, an editor `drawer-{name}-form`. */
	type Editor = Field | 'description';

	let story = $state<Story | null>(null);
	let readiness = $state<Readiness | null>(null);
	let decisions = $state<RequiredDecision[]>([]);
	let comments = $state<Comment[]>([]);
	let activity = $state<ActivityEntry[]>([]);
	let epics = $state<Epic[]>([]);
	let initiatives = $state<Initiative[]>([]);
	let panel = $state<HTMLElement>();
	let sprints = $state<Sprints | null>(null);
	/** The active sprint's goals: the Goal field's choices. Without an active sprint the field is absent. */
	const activeGoals = $derived(sprints?.active?.sprint.goals ?? null);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	/** A reload failed after the story had loaded: the content stays, the notice offers a retry. */
	let reloadFailed = $state(false);
	let busy = $state(false);
	/** A move refused for readiness reasons, waiting for an override reason. */
	let override = $state<(PendingMove & { reasons: Reason[] }) | null>(null);

	// Drafts follow the saved value through a primitive, so a reload that returns the same value keeps an unsaved edit.
	const savedTitle = $derived(story?.title ?? '');
	const savedOwner = $derived(story?.owner ?? '');
	const savedLabels = $derived(story?.labels.join(', ') ?? '');
	const savedEpic = $derived(story?.epic ?? '');
	let title = $derived(savedTitle);
	let owner = $derived(savedOwner);
	let labels = $derived(savedLabels);
	let epic = $derived(savedEpic);
	/** The goal chosen in the Goal editor, as the select's value ('' while none is chosen). */
	let goalChoice = $state('');
	/** The fields whose editor is open; every other field shows its value with a pen beside it. */
	const editing = new SvelteSet<Field>();
	/** The status button chose done: the resolution dialog is open (entering done always carries one, kb/ADR-0002 R4). */
	let askResolution = $state(false);
	let discussion = $state<'comments' | 'activity'>('comments');
	let discussionTab = $state<string>();

	let editingDescription = $state(false);
	let description = $state('');
	/** The version (and description) the open editor started from. A live refetch moves `version` under the draft; saving
	 *  with the base turns a change to the description made meanwhile into a conflict instead of silently overwriting it,
	 *  while a change to any other field advances the base (`followBase`). */
	let descriptionBase: EditorBase = { version: 0, text: '' };
	let descriptionField = $state<HTMLTextAreaElement | null>(null);
	const descriptionUploads = new ImageUploads(
		() => description,
		(t) => (description = t)
	);
	let blocker = $state('');
	let showClosed = $state(false);
	let decision = $state({ title: '', decider: '', due: '' });
	/** The "Require a decision" form's idempotency key; the drawer is remounted per story, so it needs no reset on change. */
	let decisionDraft = newDraftKey();

	const shownDecisions = $derived(filterDecisions(decisions, { closed: showClosed }));
	const closedCount = $derived(decisions.filter((d) => d.closed).length);
	const parsedLabels = $derived(labelList(labels));
	const me = $derived(session.me?.email ?? null);

	let heading = $state<HTMLElement>();
	let latest = 0;

	/** Every write is followed by this, so each `version` sent is the one the server last returned. */
	async function reload() {
		const key = storyKey;
		const seq = ++latest;
		try {
			const loaded = unwrap(api.GET('/api/v1/stories/{key}', { params: { path: { key } } }));
			const [s, r, d, c, a, sp] = await Promise.all([
				loaded,
				unwrap(api.GET('/api/v1/stories/{key}/ready', { params: { path: { key } } })),
				unwrap(api.GET('/api/v1/decisions', { params: { query: { on: key } } })),
				unwrap(api.GET('/api/v1/stories/{key}/comments', { params: { path: { key } } })),
				// The activity log is keyed by the canonical key only: the URL's key may be an external reference (SCRUM-11),
				// which the story endpoints resolve and this one does not.
				loaded.then((s) =>
					unwrap(api.GET('/api/v1/activity', { params: { query: { entity_type: 'story', entity_key: s.key } } }))
				),
				// The sprints are an extra: a failure leaves the drawer without its Goal field, never without the story.
				unwrap(api.GET('/api/v1/sprints')).catch(() => null)
			]);
			if (seq !== latest) return;
			story = s;
			loadedAt = new Date();
			if (editingDescription) descriptionBase = followBase(descriptionBase, { version: s.version, text: s.description });
			readiness = r;
			decisions = d;
			comments = c;
			activity = a;
			sprints = sp;
			loadError = null;
			reloadFailed = false;
		} catch (e) {
			if (seq !== latest) return;
			if (story) {
				notice = `Could not reload ${key}: ${errorMessage(e)}`;
				reloadFailed = true;
			} else loadError = errorMessage(e);
		}
	}

	async function loadInitiatives() {
		initiatives = await unwrap(api.GET('/api/v1/initiatives')).catch(() => []);
	}

	async function loadEpics() {
		try {
			epics = await unwrap(api.GET('/api/v1/epics'));
		} catch (e) {
			notice = `Could not load the epics: ${errorMessage(e)}`;
		}
	}

	// The layout owns the opener: it returns focus there once the drawer closes.
	onMount(() => {
		reload().then(async () => {
			await tick();
			heading?.focus();
		});
		loadEpics();
		loadInitiatives();
	});

	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
	onLive(reload);

	function labelList(text: string): string[] {
		return [...new Set(text.split(',').map((l) => l.trim()).filter(Boolean))];
	}

	/** Refusals show their reasons; a conflict reloads (a draft survives unless the server's value for its field changed);
	 *  anything else shows its message. */
	async function failed(e: unknown) {
		if (e instanceof Refused) notice = e.reasons.map((r) => fullLabel(r)).join('; ');
		else if (e instanceof Conflict) {
			notice = 'Someone else changed this story; it was reloaded. Check it and try again.';
			await reload();
		} else if (e instanceof ApiError) notice = e.message;
		else notice = errorMessage(e);
	}

	/** Whether the last `write` failed with a `Conflict`: only then is an open editor's base rebased. */
	let conflicted = false;

	async function write(action: () => Promise<unknown>): Promise<boolean> {
		conflicted = false;
		if (busy) return false;
		busy = true;
		notice = null;
		try {
			await action();
			await reload();
			return true;
		} catch (e) {
			conflicted = e instanceof Conflict;
			await failed(e);
			return false;
		} finally {
			busy = false;
		}
	}

	/** `PATCH /api/v1/stories/{key}` with the version last loaded and only the fields being changed. */
	function patch(body: Omit<EditItem, 'version'>, version?: number): Promise<boolean> {
		const s = story;
		if (!s) return Promise.resolve(false);
		return write(() =>
			unwrap(
				api.PATCH('/api/v1/stories/{key}', { params: { path: { key: s.key } }, body: { version: version ?? s.version, ...body } })
			)
		);
	}

	const editorOf = (f: Editor) => document.getElementById(`drawer-${f}-form`);

	/** The editors whose save is pending. Each is locked meanwhile, its fields and its Cancel disabled: its answer closes
	 *  it (or reloads under it), which would drop anything typed, or a draft reopened, after the save was sent. */
	const saving = new SvelteSet<Editor>();
	const locked = (f: Editor) => saving.has(f);

	async function sending<T>(f: Editor, run: () => Promise<T>): Promise<T> {
		saving.add(f);
		try {
			return await run();
		} finally {
			saving.delete(f);
		}
	}

	/** Focuses an editor's first field. Opening always does (the pen that was pressed leaves the DOM); after a failed save,
	 *  whose disabled fields may have dropped focus to the page, only if the user has not moved it elsewhere
	 *  (#lib/focus.ts). */
	async function focusEditor(f: Editor, opening = false) {
		await tick();
		const form = editorOf(f);
		if (opening || focusIsFree(form)) form?.querySelector<HTMLElement>('input, select, textarea')?.focus();
	}

	function edit(f: Field) {
		editing.add(f);
		focusEditor(f, true);
	}

	/** Removes an editor and returns focus to its pen (or add button), but only if focus was still in the editor or on
	 *  the page: a save can finish after the user moved on to another field. */
	async function leave(f: Editor, remove: () => void) {
		const free = focusIsFree(editorOf(f));
		remove();
		await tick();
		if (free) document.getElementById(`drawer-${f}-edit`)?.focus();
	}

	const close = (f: Field) => leave(f, () => editing.delete(f));

	/** Cancel drops the field's draft: reopening starts from the saved value. */
	function cancel(f: Editor) {
		if (f === 'description') return stopDescription();
		if (f === 'title') title = savedTitle;
		else if (f === 'owner') owner = savedOwner;
		else if (f === 'labels') labels = savedLabels;
		else if (f === 'epic') epic = savedEpic;
		else if (f === 'goal') goalChoice = '';
		else if (f === 'blocker') blocker = '';
		else if (f === 'decision') {
			decision = { title: '', decider: '', due: '' };
			decisionDraft = newDraftKey();
		}
		close(f);
	}

	/** Escape in an open editor cancels it (the drawer's own Escape ignores form fields). */
	function cancelOnEscape(f: Editor) {
		return (e: KeyboardEvent) => {
			if (e.key !== 'Escape' || e.defaultPrevented || locked(f)) return;
			e.preventDefault();
			cancel(f);
		};
	}

	/** Saves when the field changed, and closes its editor unless the save failed. */
	async function save(f: Field, changed: boolean, body: Omit<EditItem, 'version'>) {
		if (!changed) return close(f);
		if (await sending(f, () => patch(body))) close(f);
		else focusEditor(f);
	}

	function saveTitle(e: SubmitEvent) {
		e.preventDefault();
		if (title.trim()) save('title', title.trim() !== savedTitle, { title: title.trim() });
	}

	function saveOwner(e: SubmitEvent) {
		e.preventDefault();
		const o = owner.trim();
		save('owner', o !== savedOwner, { owner: o || null });
	}

	function saveLabels(e: SubmitEvent) {
		e.preventDefault();
		save('labels', parsedLabels.join(', ') !== savedLabels, { labels: parsedLabels });
	}

	function saveEpic(e: SubmitEvent) {
		e.preventDefault();
		save('epic', epic !== savedEpic, { epic: epic || null });
	}

	/** Links the story to the chosen goal (`POST /goals/{id}/links`); the story's goals come back with the reload. Goal
	 *  links never touch delivery or readiness (kb/ADR-0014 R4). */
	async function saveGoal(e: SubmitEvent) {
		e.preventDefault();
		const s = story;
		const id = Number(goalChoice);
		if (!s || !goalChoice) return close('goal');
		const ok = await sending('goal', () =>
			write(() => unwrap(api.POST('/api/v1/goals/{id}/links', { params: { path: { id } }, body: { items: [s.key] } })))
		);
		if (ok) {
			goalChoice = '';
			close('goal');
		} else focusEditor('goal');
	}

	/** `DELETE /goals/{id}/links/{key}`: only a link the story itself holds; one through its epic or initiative is
	 *  unlinked from the sprint page. */
	async function unlinkGoal(id: number) {
		const s = story;
		if (!s) return;
		const ok = await write(() =>
			unwrap(api.DELETE('/api/v1/goals/{id}/links/{key}', { params: { path: { id, key: s.key } } }))
		);
		// The removed goal's button left the DOM; keep keyboard focus in the field.
		if (ok) {
			await tick();
			document.getElementById('drawer-goal-edit')?.focus();
		}
	}

	/** When the drawer last loaded: the urgent line's "N d ago" counts from it. */
	let loadedAt = $state(new Date());
	/** Whether the "Mark urgent" dialog is open, and the reason a failed mark left in it. */
	let marking = $state(false);
	let markError = $state<string | null>(null);
	/** A source's title, when the drawer has it (the epics and initiatives lists); otherwise the line shows its key and type only. */
	const titleOf = (r: ItemRef): string | undefined =>
		r.item === 'epic'
			? epics.find((e) => e.key === r.key)?.title
			: r.item === 'initiative'
				? initiatives.find((i) => i.key === r.key)?.title
				: undefined;

	const menuItems = $derived<MenuItem[]>([
		// A done story cannot be urgent: its mark can only be cleared.
		...(story?.delivery === 'done' ? [] : [{ label: 'Mark urgent…', onselect: openMark, disabled: busy }]),
		...(story?.urgent_mark ? [{ label: 'Clear urgent mark', onselect: () => void clearUrgent(), disabled: busy }] : [])
	]);

	function openMark() {
		markError = null;
		marking = true;
	}

	async function markUrgent(reason: string | null) {
		const s = story;
		if (!s) return;
		const ok = await write(() =>
			unwrap(api.POST('/api/v1/stories/{key}/urgent', { params: { path: { key: s.key } }, body: { reason } }))
		);
		if (ok) {
			marking = false;
			await refocusActions();
		} else {
			markError = notice;
			notice = null;
		}
	}

	async function clearUrgent() {
		const s = story;
		if (!s) return;
		if (await write(() => unwrap(api.DELETE('/api/v1/stories/{key}/urgent', { params: { path: { key: s.key } } })))) {
			await refocusActions();
		} else {
			// The Clear button that was pressed is still there: keep the keyboard on it.
			await tick();
			if (focusIsFree(null)) panel?.querySelector<HTMLElement>(`button[aria-label="Clear urgent mark on ${s.key}"]`)?.focus();
		}
	}

	/** The control that opened a write (a dialog, the expired or done line's buttons) is gone: focus goes to the drawer's actions menu, unless the user moved it. */
	async function refocusActions() {
		await tick();
		if (focusIsFree(null)) (panel?.querySelector<HTMLElement>('[data-control="story-actions"]') ?? heading)?.focus();
	}

	async function saveCriteria(criteria: Story['criteria']) {
		if (await sending('criteria', () => patch({ criteria }))) close('criteria');
		else focusEditor('criteria');
	}

	function startDescription() {
		descriptionUploads.reset();
		description = story?.description ?? '';
		descriptionBase = { version: story?.version ?? 0, text: story?.description ?? '' };
		editingDescription = true;
		focusEditor('description', true);
	}

	const stopDescription = () =>
		leave('description', () => {
			editingDescription = false;
			descriptionUploads.reset();
		});

	async function saveDescription(e: SubmitEvent) {
		e.preventDefault();
		if (descriptionUploads.busy) return;
		if (await sending('description', () => patch({ description, ...descriptionUploads.body() }, descriptionBase.version))) {
			descriptionUploads.reset();
			stopDescription();
			return;
		}
		focusEditor('description');
		// Only a conflict (which reloaded and told the user) rebases on what is loaded now, so trying again can succeed. Any
		// other failure keeps the base: a retry must not overwrite a colleague's text the user has not been shown.
		if (conflicted && story) descriptionBase = rebase({ version: story.version, text: story.description });
	}

	/** The status button's choice: the same request, refusal and override flow as the board (`#lib/moves.ts`). Done asks
	 *  for the resolution first. ActionsMenu refocused the status button before this ran, so the dialog and the override
	 *  prompt return focus to it. */
	function choose(to: Column) {
		if (!story) return;
		const step = moveStep(story, to);
		if (step === 'resolve') askResolution = true;
		else if (step === 'move') runMove({ to });
	}

	async function runMove(m: PendingMove, overrideReason?: string) {
		const s = story;
		if (!s || busy) return;
		busy = true;
		notice = null;
		try {
			const out = await postMove(s, m.to, { resolution: m.resolution, overrideReason });
			if (out.kind === 'override') override = { ...m, reasons: out.reasons };
			else {
				override = null;
				if (out.kind === 'notice') notice = out.notice;
				else await reload();
			}
		} catch (err) {
			override = null;
			await failed(err);
		} finally {
			busy = false;
		}
	}

	async function addBlocker(e: SubmitEvent) {
		e.preventDefault();
		const b = blocker.trim();
		if (!b) return;
		const ok = await sending('blocker', () =>
			write(() => unwrap(api.POST('/api/v1/stories/{key}/blocks', { params: { path: { key: storyKey } }, body: { blocker: b } })))
		);
		if (ok) {
			blocker = '';
			close('blocker');
		} else focusEditor('blocker');
	}

	/** `DELETE /stories/{key}/blocks/{blocker}`. Removing an edge someone else already removed is a no-op on the server, so
	 *  a stale list never errors; `write` reloads the drawer. */
	async function removeBlocker(b: string) {
		const ok = await write(() =>
			unwrap(api.DELETE('/api/v1/stories/{key}/blocks/{blocker}', { params: { path: { key: storyKey, blocker: b } } }))
		);
		// The removed row's button left the DOM; keep keyboard focus in the section.
		if (ok) {
			await tick();
			(
				document.getElementById('drawer-blocker') ??
				document.getElementById('drawer-blocker-edit') ??
				document.getElementById('drawer-blockers')
			)?.focus();
		}
		return ok;
	}

	async function requireDecision(e: SubmitEvent) {
		e.preventDefault();
		const d = { ...decision };
		if (!d.title.trim() || !d.decider.trim()) return;
		const ok = await sending('decision', () =>
			write(() =>
				unwrap(
					api.POST('/api/v1/decisions', {
						body: {
							idempotency_key: draftKey(decisionDraft, JSON.stringify([storyKey, d.title.trim(), d.decider.trim(), d.due])),
							on: storyKey,
							title: d.title.trim(),
							decider: d.decider.trim(),
							due: d.due || null
						}
					})
				)
			)
		);
		// Only if the form still holds what was sent (its inputs are disabled while the create is pending, so this is a guard).
		if (ok && decision.title === d.title && decision.decider === d.decider && decision.due === d.due) {
			decision = { title: '', decider: '', due: '' };
			decisionDraft = newDraftKey();
			close('decision');
		} else if (!ok) focusEditor('decision');
	}

	function posted(thread: Comment[]) {
		comments = thread;
		reload();
	}

	function keydown(e: KeyboardEvent) {
		// The override prompt, the resolution dialog and the mark dialog render outside the aside, so their Escape never reaches this handler;
		// the `!override` check is defensive only.
		// Escape in a form field is the field's own (cancel an edit); it never closes the drawer over an unsaved draft.
		const t = e.target;
		const inField = t instanceof HTMLElement && t.matches('textarea, input, select');
		if (e.key === 'Escape' && !override && !e.defaultPrevented && !inField) {
			e.preventDefault();
			onclose();
		}
	}
</script>

{#snippet pen(f: Field | 'description', what: string, onclick: () => void)}
	<button type="button" class="pen" id="drawer-{f}-edit" aria-label="Edit {what}" title="Edit {what}" {onclick}
		><span aria-hidden="true">✎</span></button
	>
{/snippet}

<!-- The explicit role is kept for selectors and tests that look the drawer up by it. -->
<!-- Escape anywhere inside closes it, so the listener sits on the panel. -->
<!-- svelte-ignore a11y_no_redundant_roles, a11y_no_noninteractive_element_interactions -->
<aside class="drawer" role="complementary" aria-label="Story {storyKey}" data-testid="story-drawer" bind:this={panel} onkeydown={keydown}>
	<header class="top">
		<h2 tabindex="-1" bind:this={heading}>
			<span class="key">{#if story}<Glyph level={levelOfKind(story.kind)} />{/if}{storyKey}</span>
			{#if story}<span class="chip">{story.kind}</span>{/if}
			{#if story?.external_ref}<span class="muted" title="External reference">from {story.external_ref}</span>{/if}
		</h2>
		{#if story}
			{#if story.epic}
				{@const e = story.epic}
				{@const epicTitle = epics.find((ep) => ep.key === e)?.title}
				<a class="epic-link" href="/epics/{encodeURIComponent(e)}"
					><Glyph level="epic" /><span class="key">{e}</span>{#if epicTitle}&nbsp;{epicTitle}{/if}</a
				>
			{:else}
				<span class="muted epic-link">No epic</span>
			{/if}
		{/if}
		<!-- The drawer covers the header's Report button at every width; the story never prefills the epic chip. -->
		<button type="button" class="report-here" onclick={() => openReport({ ...reportContext(page.url), epic: null })}>{reportLabel()}</button>
		<button type="button" class="close" aria-label="Close story {storyKey}" onclick={onclose}>×</button>
	</header>

	{#if loadError}
		<p class="error" role="alert">Could not load {storyKey}: {loadError}</p>
	{:else if !story || !readiness}
		<p class="muted">Loading…</p>
	{:else}
		{@const s = story}
		{@const r = readiness}

		{#if notice}
			<Notice
				onretry={reloadFailed ? reload : undefined}
				ondismiss={() => {
					notice = null;
					reloadFailed = false;
				}}>{notice}</Notice
			>
		{/if}

		{#if editing.has('title')}
			<form class="title-form" id="drawer-title-form" onsubmit={saveTitle} onkeydown={cancelOnEscape('title')}>
				<label for="drawer-title" class="visually-hidden">Title</label>
				<input id="drawer-title" bind:value={title} required disabled={locked('title')} />
				<button type="button" disabled={locked('title')} onclick={() => cancel('title')}>Cancel</button>
				<button type="submit" disabled={busy || !title.trim()}>Save</button>
			</form>
		{:else}
			<div class="title-view">
				<p class="title">{s.title}</p>
				{@render pen('title', 'title', () => edit('title'))}
			</div>
		{/if}

		<div class="status-row">
			<ActionsMenu
				label="Move {s.key}, now {COLUMN_LABEL[s.delivery]}"
				class="status-trigger"
				items={COLUMNS.filter((c) => moveStep(s, c) !== 'none').map((c) => ({
					label: c === 'done' && s.delivery === 'done' ? 'Change resolution' : `Move to ${COLUMN_LABEL[c]}`,
					disabled: busy,
					onselect: () => choose(c)
				}))}
			>
				{#snippet trigger()}{COLUMN_LABEL[s.delivery]} <span aria-hidden="true">▾</span>{/snippet}
			</ActionsMenu>
			{#if s.delivery === 'done' && s.resolution}<span class="chip">{s.resolution}</span>{/if}
			<span class="owner"><span class="muted">Owner</span> <Avatar email={s.owner} /> {s.owner ? short(s.owner) : 'unowned'}</span>
			{#if menuItems.length}
				<ActionsMenu label="More actions for {s.key}" control="story-actions" items={menuItems} />
			{/if}
		</div>

		<UrgentLine
			itemKey={s.key}
			mark={s.urgent_mark?.done ? null : s.urgent_mark}
			sources={s.urgent_sources}
			{titleOf}
			now={loadedAt}
			{busy}
			onremark={openMark}
			onclear={() => void clearUrgent()}
		/>
		{#if s.urgent_mark?.done}
			<p class="urgent-done muted" data-testid="urgent-done">
				<span>Urgent mark · done · {expiry(s.urgent_mark, loadedAt)}</span>
				<span aria-hidden="true">·</span>
				<button type="button" class="ghost" aria-label="Clear urgent mark on {s.key}" onclick={() => void clearUrgent()} disabled={busy}>Clear</button>
			</p>
		{/if}

		<section class="readiness {r.ready ? 'ready' : 'blocked'}" aria-labelledby="drawer-readiness">
			<h3 id="drawer-readiness">{r.ready ? 'Ready to start' : 'Not ready'}</h3>
			{#if !r.ready && r.reasons.length > 0}
				<ul class="reasons">
					{#each r.reasons as reason, i (i)}
						<li>{fullLabel(reason)}</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section aria-labelledby="drawer-criteria">
			<div class="section-head">
				<h3 id="drawer-criteria">Acceptance criteria</h3>
				{#if !editing.has('criteria')}{@render pen('criteria', 'acceptance criteria', () => edit('criteria'))}{/if}
			</div>
			{#if editing.has('criteria')}
				<!-- svelte-ignore a11y_no_static_element_interactions -->
				<div id="drawer-criteria-form" onkeydown={cancelOnEscape('criteria')}>
					<CriteriaEditor
						storyKey={s.key}
						criteria={s.criteria}
						{busy}
						locked={locked('criteria')}
						onsave={saveCriteria}
						oncancel={() => cancel('criteria')}
					/>
				</div>
			{:else if s.criteria.length === 0}
				<p class="muted">No acceptance criteria.</p>
			{:else}
				<ul class="criteria-view">
					{#each s.criteria as c, i (i)}
						<li class:done={c.done}>
							<span class="tick" aria-hidden="true">{c.done ? '☑' : '☐'}</span><span class="visually-hidden"
								>{c.done ? 'done: ' : 'open: '}</span
							>{c.text}
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section aria-labelledby="drawer-decisions">
			<h3 id="drawer-decisions">Required decisions</h3>
			{#if closedCount > 0}
				<label class="toggle"><input type="checkbox" bind:checked={showClosed} /> show closed ({closedCount})</label>
			{/if}
			<DecisionList decisions={shownDecisions} onchanged={reload} />
			{#if editing.has('decision')}
				<form class="inline-form" id="drawer-decision-form" onsubmit={requireDecision} onkeydown={cancelOnEscape('decision')}>
					<h4>Require a decision</h4>
					<label>Title <input bind:value={decision.title} required disabled={busy} /></label>
					<label>Decider <input type="email" bind:value={decision.decider} required disabled={busy} placeholder="name@example.com" /></label>
					<label>Due <input type="date" bind:value={decision.due} disabled={busy} /></label>
					<div class="actions">
						<button type="button" disabled={locked('decision')} onclick={() => cancel('decision')}>Cancel</button>
						<button type="submit" disabled={busy || !decision.title.trim() || !decision.decider.trim()}
							>Require decision</button
						>
					</div>
				</form>
			{:else}
				<!-- Disabled while a write is pending: the form's fields are, so focus could not enter it. -->
				<button type="button" class="add" id="drawer-decision-edit" disabled={busy} onclick={() => edit('decision')}>+ Require a decision</button>
			{/if}
		</section>

		<section aria-labelledby="drawer-description">
			<div class="section-head">
				<h3 id="drawer-description">Description</h3>
				{#if !editingDescription}{@render pen('description', 'description', startDescription)}{/if}
			</div>
			{#if editingDescription}
				<form class="stack" id="drawer-description-form" onsubmit={saveDescription} onkeydown={cancelOnEscape('description')}>
					<textarea
						bind:this={descriptionField}
						bind:value={description}
						disabled={locked('description')}
						rows="10"
						aria-label="description (Markdown)"
						{@attach attachable(descriptionUploads)}
					></textarea>
					<ImageUploadsRow uploads={descriptionUploads} field={descriptionField} disabled={locked('description')} />
					<div class="actions">
						<button type="button" disabled={locked('description')} onclick={stopDescription}>Cancel</button>
						<button type="submit" disabled={busy || descriptionUploads.busy}
							>{descriptionUploads.busy ? 'Uploading…' : 'Save description'}</button
						>
					</div>
				</form>
			{:else if s.description.trim()}
				<Brief markdown={s.description} />
			{:else}
				<p class="muted">No description.</p>
			{/if}
			<Attachments itemKey={s.key} />
		</section>

		<section class="fields" aria-labelledby="drawer-details">
			<h3 id="drawer-details">Details</h3>
			<div class="field">
				{#if editing.has('owner')}
					<label for="drawer-owner" class="name">Owner</label>
					<form id="drawer-owner-form" onsubmit={saveOwner} onkeydown={cancelOnEscape('owner')}>
						<input id="drawer-owner" type="email" bind:value={owner} disabled={locked('owner')} placeholder="unowned" />
						<button type="button" disabled={locked('owner')} onclick={() => cancel('owner')}>Cancel</button>
						<button type="submit" disabled={busy}>Save</button>
					</form>
				{:else}
					<span class="name">Owner</span>
					<div class="value">
						<Avatar email={s.owner} />
						<span class:muted={!s.owner}>{s.owner ?? 'unowned'}</span>
						{@render pen('owner', 'owner', () => edit('owner'))}
						{#if me && s.owner !== me}
							<button type="button" class="link" disabled={busy} onclick={() => patch({ owner: me })}>Assign to me</button>
						{/if}
						{#if s.owner}
							<button type="button" class="link" disabled={busy} onclick={() => patch({ owner: null })}>Unassign</button>
						{/if}
					</div>
				{/if}
			</div>

			<div class="field">
				{#if editing.has('labels')}
					<label for="drawer-labels" class="name">Labels</label>
					<form id="drawer-labels-form" onsubmit={saveLabels} onkeydown={cancelOnEscape('labels')}>
						<input id="drawer-labels" bind:value={labels} disabled={locked('labels')} placeholder="comma, separated" />
						<button type="button" disabled={locked('labels')} onclick={() => cancel('labels')}>Cancel</button>
						<button type="submit" disabled={busy}>Save</button>
					</form>
				{:else}
					<span class="name">Labels</span>
					<div class="value">
						{#each s.labels as l, i (i)}<span class="chip">{l}</span>{:else}<span class="muted">none</span>{/each}
						{@render pen('labels', 'labels', () => edit('labels'))}
					</div>
				{/if}
			</div>

			<div class="field">
				{#if editing.has('epic')}
					<label for="drawer-epic" class="name">Epic</label>
					<form id="drawer-epic-form" onsubmit={saveEpic} onkeydown={cancelOnEscape('epic')}>
						<select id="drawer-epic" bind:value={epic} disabled={locked('epic')}>
							<option value="">none</option>
							{#each epics as ep (ep.key)}
								<option value={ep.key}>{ep.key} {ep.title}</option>
							{/each}
							{#if s.epic && !epics.some((ep) => ep.key === s.epic)}
								<option value={s.epic}>{s.epic}</option>
							{/if}
						</select>
						<button type="button" disabled={locked('epic')} onclick={() => cancel('epic')}>Cancel</button>
						<button type="submit" disabled={busy}>Save</button>
					</form>
				{:else}
					<span class="name">Epic</span>
					<div class="value">
						{#if s.epic}
							{@const e = s.epic}
							<a class="epic-link" href="/epics/{encodeURIComponent(e)}"
								><span class="key">{e}</span> {epics.find((ep) => ep.key === e)?.title ?? ''}</a
							>
						{:else}
							<span class="muted">none</span>
						{/if}
						{@render pen('epic', 'epic', () => edit('epic'))}
					</div>
				{/if}
			</div>

			{#if activeGoals}
				<!-- The active sprint's goals this story serves (kb/ADR-0014 R3), as the server computed them. -->
				<div class="field">
					{#if editing.has('goal')}
						<label for="drawer-goal" class="name">Goal</label>
						<form id="drawer-goal-form" onsubmit={saveGoal} onkeydown={cancelOnEscape('goal')}>
							<select id="drawer-goal" bind:value={goalChoice} disabled={locked('goal')}>
								<option value="">choose a goal</option>
								{#each activeGoals.filter((g) => !s.goals.some((sg) => sg.id === g.id && sg.via === 'story')) as g (g.id)}
									<option value={String(g.id)}>{g.outcome}</option>
								{/each}
							</select>
							<button type="button" disabled={locked('goal')} onclick={() => cancel('goal')}>Cancel</button>
							<button type="submit" disabled={busy}>Save</button>
						</form>
					{:else}
						<span class="name">Goal</span>
						<div class="value">
							{#each s.goals as g (g.id + g.via)}
								<span class="chip">{g.outcome}{#if g.via !== 'story'}<span class="muted">&nbsp;via {g.via}</span>{/if}</span>
								{#if g.via === 'story'}
									<button
										type="button"
										class="link"
										aria-label="Unlink {s.key} from {g.outcome}"
										disabled={busy}
										onclick={() => unlinkGoal(g.id)}>Unlink</button
									>
								{/if}
							{:else}
								<span class="muted">none</span>
							{/each}
							{@render pen('goal', 'goal', () => edit('goal'))}
						</div>
					{/if}
				</div>
			{/if}
		</section>

		<section aria-labelledby="drawer-blockers">
			<h3 id="drawer-blockers" tabindex="-1">Blocked by</h3>
			{#if s.blocked_by.length === 0}
				<p class="muted">Nothing blocks this story.</p>
			{:else}
				<ul class="blockers">
					{#each s.blocked_by as b (b)}
						<li>
							<a href={storyHref(page.url, b)}><Glyph level="story" />{b}</a>
							<button type="button" aria-label="Remove block by {b}" disabled={busy} onclick={() => removeBlocker(b)}>Remove</button>
						</li>
					{/each}
				</ul>
			{/if}
			{#if editing.has('blocker')}
				<form id="drawer-blocker-form" onsubmit={addBlocker} onkeydown={cancelOnEscape('blocker')}>
					<label for="drawer-blocker">Add blocker</label>
					<input id="drawer-blocker" bind:value={blocker} disabled={locked('blocker')} placeholder="story key" />
					<button type="button" disabled={locked('blocker')} onclick={() => cancel('blocker')}>Cancel</button>
					<button type="submit" disabled={busy || !blocker.trim()}>Add</button>
				</form>
			{:else}
				<button type="button" class="add" id="drawer-blocker-edit" onclick={() => edit('blocker')}>+ Add blocker</button>
			{/if}
		</section>

		<section aria-labelledby="drawer-prs">
			<h3 id="drawer-prs">Pull requests</h3>
			{#if s.prs.length === 0}
				<p class="muted">Linked pull requests arrive with the GitHub webhook.</p>
			{:else}
				<ul class="prs" data-testid="drawer-prs">
					{#each s.prs as pr (pr.repo + '#' + pr.number)}
						<li>
							{#if safePrUrl(pr.url)}
								<a
									href={safePrUrl(pr.url)}
									target="_blank"
									rel="noopener noreferrer"
									aria-label="pull request {pr.repo}#{pr.number}, {pr.status}"
									><span class="key">{pr.repo}#{pr.number}</span> {pr.title}</a
								>
							{:else}
								<span><span class="key">{pr.repo}#{pr.number}</span> {pr.title}</span>
							{/if}
							<span class="chip {pr.status}">{PR_LABEL[pr.status]}</span>
							<span class="muted">{age(pr.updated_at)}</span>
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section aria-labelledby="drawer-discussion">
			<h3 id="drawer-discussion">Discussion</h3>
			<Toggle
				mode="tabs"
				label="Discussion"
				options={[
					{ value: 'comments', label: 'Comments' },
					{ value: 'activity', label: 'Activity' }
				]}
				value={discussion}
				onchange={(v) => {
					discussion = v;
				}}
				panel="drawer-discussion-panel"
				bind:selectedTabId={discussionTab}
			/>
			<div role="tabpanel" tabindex="0" id="drawer-discussion-panel" class="tabpanel" aria-labelledby={discussionTab} data-tab={discussion}>
				<!-- Both stay mounted: switching tabs must not lose a half-written comment or its retry state. -->
				<div hidden={discussion !== 'comments'}>
					<Comments itemKey={s.key} {comments} onposted={posted} />
				</div>
				<div hidden={discussion !== 'activity'}>
					<ActivityList entries={activity} />
				</div>
			</div>
		</section>
	{/if}
</aside>

{#if askResolution && story}
	{@const s = story}
	<Dialog title={s.delivery === 'done' ? `Change the resolution of ${s.key}` : `Move ${s.key} to done`} onclose={() => (askResolution = false)}>
		<p>How was it resolved?</p>
		<div class="actions">
			{#each resolutionsFor(s.delivery, s.resolution ?? null) as res (res)}
				<button
					type="button"
					disabled={busy}
					onclick={() => {
						askResolution = false;
						runMove({ to: 'done', resolution: res });
					}}>{res}</button
				>
			{/each}
		</div>
	</Dialog>
{/if}

{#if marking && story}
	<MarkUrgentDialog
		item={story.key}
		initial={story.urgent_mark?.reason ?? null}
		{busy}
		error={markError}
		onsubmit={markUrgent}
		oncancel={() => {
			if (!busy) marking = false;
		}}
	/>
{/if}

{#if override && story}
	{@const o = override}
	<OverridePrompt
		storyKey={story.key}
		target={COLUMN_LABEL[o.to]}
		reasons={o.reasons}
		{busy}
		onconfirm={(why) => runMove(o, why)}
		oncancel={() => (override = null)}
	/>
{/if}

<style>
	.urgent-done {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 10px;
		margin: 0;
		font-size: 14px;
	}
	.drawer {
		position: fixed;
		top: 0;
		right: 0;
		bottom: 0;
		z-index: 10;
		display: grid;
		/* One column no wider than the panel: an input's intrinsic width must not widen the track into a horizontal scroll. */
		grid-template-columns: minmax(0, 1fr);
		align-content: start;
		gap: 16px;
		width: min(560px, 100%);
		box-sizing: border-box;
		padding: 16px 20px 28px;
		overflow-x: hidden;
		overflow-y: auto;
		scrollbar-width: thin;
		border-left: 1px solid var(--line);
		background: var(--background);
		box-shadow: var(--shadow);
	}
	.top {
		display: flex;
		justify-content: space-between;
		align-items: center;
		padding: 0;
		border: 0;
		background: none;
	}
	h2 {
		display: flex;
		gap: 0.5rem;
		align-items: center;
		margin: 0;
		font-size: 18px;
	}
	h3 {
		margin: 0 0 6px;
		font-size: 13px;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--muted);
	}
	.title-form input {
		flex: 1;
		min-width: 0;
		font: inherit;
		font-size: 24px;
		font-weight: 600;
	}
	.title-view {
		display: flex;
		gap: 6px;
		align-items: baseline;
	}
	.title {
		margin: 0;
		font-size: 24px;
		font-weight: 600;
		line-height: 1.25;
		overflow-wrap: anywhere;
	}
	.section-head {
		display: flex;
		gap: 6px;
		align-items: baseline;
	}
	/* The pen, the "+ add" buttons and the quick owner actions read as quiet text until hovered. */
	.pen,
	.add,
	.link {
		padding: 0 4px;
		border: 0;
		background: none;
		color: var(--muted);
		font-size: 14px;
		cursor: pointer;
	}
	.pen:hover,
	.add:hover,
	.link:hover:not(:disabled) {
		color: var(--foreground);
	}
	.link {
		text-decoration: underline;
	}
	.add {
		justify-self: start;
		margin-top: 4px;
		padding: 0;
	}
	/* The drawer hides horizontal overflow, so a long unbroken token (a URL, an identifier) wraps instead of clipping. */
	.criteria-view,
	.blockers,
	.prs {
		overflow-wrap: anywhere;
	}
	.criteria-view {
		display: grid;
		gap: 0.25rem;
		margin: 0;
		padding: 0;
		list-style: none;
		font-size: 14px;
	}
	.criteria-view .tick {
		margin-right: 0.4rem;
	}
	.criteria-view .done {
		color: var(--muted);
	}
	.status-row {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
	}
	.status-row .owner {
		display: inline-flex;
		gap: 6px;
		align-items: center;
		margin-left: auto;
	}
	.status-row :global(.status-trigger) {
		height: 28px;
	}
	.readiness {
		padding: 10px 12px;
		border-radius: 6px;
	}
	.readiness.ready {
		background: var(--ready-soft);
	}
	.readiness.blocked {
		background: var(--warn-soft);
	}
	.readiness h3 {
		margin: 0;
		color: var(--foreground);
	}
	.readiness .reasons {
		margin: 6px 0 0;
	}
	.tabpanel {
		padding-top: 10px;
	}
	h4 {
		width: 100%;
		margin: 0;
		font-size: 14px;
	}
	.chip {
		padding: 0 0.4rem;
		border: 1px solid var(--line);
		border-radius: 999px;
		font-size: 13px;
		font-weight: 400;
		color: var(--muted);
	}
	.top .epic-link {
		min-width: 0;
		margin-left: auto;
		margin-right: 8px;
		overflow: hidden;
		font-size: 14px;
		white-space: nowrap;
		text-overflow: ellipsis;
	}
	.report-here {
		margin-left: auto;
		margin-right: 8px;
	}
	.top .epic-link + .report-here {
		margin-left: 0;
	}
	.close {
		font-size: 18px;
		line-height: 1;
	}
	.fields {
		display: grid;
		gap: 0.4rem;
	}
	.field {
		display: grid;
		grid-template-columns: 4rem minmax(0, 1fr);
		gap: 0.4rem;
		align-items: center;
		min-height: 32px;
	}
	.field .value {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		align-items: center;
		min-width: 0;
		overflow-wrap: anywhere;
	}
	form {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
		align-items: center;
	}
	.fields .name {
		font-size: 14px;
		color: var(--muted);
	}
	.fields input,
	.fields select {
		flex: 1;
		min-width: 0;
		font: inherit;
	}
	.stack {
		display: grid;
	}
	textarea {
		font: inherit;
		font-family: var(--mono);
	}
	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.5rem;
	}
	.reasons,
	.blockers {
		margin: 0 0 0.5rem;
		padding-left: 1.2rem;
		font-size: 14px;
	}
	.toggle {
		display: block;
		margin-bottom: 0.5rem;
		font-size: 14px;
	}
	.inline-form {
		gap: 0.4rem 0.75rem;
		align-items: end;
		margin-top: 0.5rem;
		padding: 0.6rem;
		border-radius: 6px;
		background: var(--lighter-background);
	}
	.inline-form .actions {
		margin-left: auto;
	}
	.inline-form label {
		display: grid;
		gap: 0.2rem;
		font-size: 14px;
	}
	.prs {
		display: grid;
		gap: 0.4rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.prs li {
		display: flex;
		flex-wrap: wrap;
		gap: 0.2rem 0.5rem;
		align-items: baseline;
	}
	.chip.open {
		color: var(--pr-open);
	}
	.chip.merged {
		color: var(--pr-merged);
	}
	.chip.draft {
		color: var(--muted);
	}
	.chip.closed {
		color: var(--muted);
	}
	.muted {
		margin: 0;
		color: var(--muted);
		font-size: 14px;
	}
	.error {
		color: var(--danger);
	}
</style>
