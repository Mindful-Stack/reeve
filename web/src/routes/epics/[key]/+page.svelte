<script lang="ts">
	import DueShortcuts from '#lib/components/DueShortcuts.svelte';
	import Picker from '#lib/ui/Picker.svelte';
	import { keyed } from '#lib/picker.ts';
	import Glyph from '#lib/components/Glyph.svelte';
	import GoalLinkButton from '#lib/components/GoalLinkButton.svelte';
	import { levelOfKind } from '#lib/glyphs.ts';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import type { ItemRef, ActivityEntry, EditItem, Epic, Initiative, RequiredDecision, Story, StoryWithReadiness, Finding, Comment } from '#lib/api/types.ts';
	import Findings from '#lib/components/Findings.svelte';
	import Comments from '#lib/components/Comments.svelte';
	import { COLUMN_LABEL, fullLabel, storyStatus } from '#lib/board.ts';
	import Brief from '#lib/components/Brief.svelte';
	import CriteriaEditor from '#lib/components/CriteriaEditor.svelte';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import StoryForm from '#lib/components/StoryForm.svelte';
	import ActivityList from '#lib/components/ActivityList.svelte';
	import { blocking } from '#lib/decisions.ts';
	import { everyVisible, freshNote, live, onLive } from '#lib/events.svelte.ts';
	import ActionsMenu, { type MenuItem } from '#lib/ui/ActionsMenu.svelte';
	import Avatar from '#lib/ui/Avatar.svelte';
	import Dialog from '#lib/ui/Dialog.svelte';
	import MarkUrgentDialog from '#lib/components/MarkUrgentDialog.svelte';
	import UrgentLine from '#lib/components/UrgentLine.svelte';
	import PromiseChip from '#lib/components/PromiseChip.svelte';
	import PromiseDialog from '#lib/components/PromiseDialog.svelte';
	import PromiseLine from '#lib/components/PromiseLine.svelte';
	import ClearPromiseDialog from '#lib/components/ClearPromiseDialog.svelte';
	import { PromiseControl } from '#lib/promiseControl.svelte.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';
	import StatusStrip from '#lib/ui/StatusStrip.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';
	import { blockerCell } from '#lib/strip.ts';
	import { epicCounts, sumCounts } from '#lib/initiative.ts';
	import { epicStripDecisions, pollsChecks } from '#lib/epic.ts';
	import { SvelteSet } from 'svelte/reactivity';
	import { errorMessage, short } from '#lib/format.ts';
	import { decisionHref, storyHref } from '#lib/drawer.ts';
	import { draftKey, newDraftKey } from '#lib/idempotency.ts';
	import { followBase, rebase, type EditorBase } from '#lib/editorBase.ts';
	import { EPIC_SAME, epicDetails, epicEdit, fieldsText, mergeDraft, type EpicDetails } from '#lib/forms.ts';
	import { tick } from 'svelte';
	import Attachments from '#lib/components/Attachments.svelte';
	import ImageUploadsRow from '#lib/components/ImageUploadsRow.svelte';
	import { ImageUploads, attachable } from '#lib/attach.svelte.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { session } from '#lib/me.svelte.ts';
	import { planningOrder } from '#lib/planning.ts';

	const key = $derived(page.params.key ?? '');

	let epic = $state<Epic | null>(null);
	let stories = $state<StoryWithReadiness[]>([]);
	/** The decisions on this epic: the Decisions section. */
	let decisions = $state<RequiredDecision[]>([]);
	/** Every decision under the epic's initiative; only those on the initiative itself reach the status strip. */
	let initiativeDecisions = $state<RequiredDecision[]>([]);
	/** Advisory, so fetched best effort: a failed read shows none rather than failing the page. */
	let findings = $state<Finding[]>([]);
	/** The item's comment thread, fetched best effort like the findings. */
	let comments = $state<Comment[]>([]);
	/** The epic's activity log, fetched best effort like the comments. */
	let activity = $state<ActivityEntry[]>([]);
	/** Every initiative, for the details picker. */
	let initiatives = $state<Initiative[]>([]);
	/** The details picker lists them in the planning page's order. */
	const orderedInitiatives = $derived(planningOrder(session.config?.horizon_buckets ?? [], initiatives));
	let loadError = $state<string | null>(null);
	/** A refetch failed after the page had loaded: the content (and its open editors) stays mounted; the banner offers a retry. */
	let staleNotice = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);

	let editing = $state(false);
	let briefDraft = $state('');
	/** The version (and brief) the open editor started from. A live refetch moves `version` under the draft; saving with
	 *  the base turns a change to the brief made meanwhile into a conflict instead of silently overwriting it, while a
	 *  change to any other field advances the base (`followBase`). */
	let briefBase: EditorBase = { version: 0, text: '' };
	let briefField = $state<HTMLTextAreaElement | null>(null);
	const briefUploads = new ImageUploads(
		() => briefDraft,
		(t) => (briefDraft = t)
	);
	let showClosed = $state(false);
	/** Whether the decisions that block nothing (accepted, and closed ones when shown) are unfolded. */
	/** The user's choice for the accepted fold; null follows the default (open when nothing blocks). */
	let accepted = $state<boolean | null>(null);
	/** The open "New story" dialog's opening, or null: a create's answer closes only the dialog it was sent from. */
	let creatingStory = $state<number | null>(null);
	let storyOpenings = 0;
	/** The open "Add decision" dialog's opening, or null: a create's answer closes only the dialog it was sent from, and
	 *  a failed one shows its reason inside that dialog, not behind the modal. */
	let addingDecision = $state<number | null>(null);
	let decisionOpenings = 0;
	/** The opening whose create is in flight, or null: only that dialog is busy, and only its own answer frees it. */
	let sendingDecision = $state<number | null>(null);
	const decisionBusy = $derived(addingDecision !== null && sendingDecision === addingDecision);
	let decisionError = $state<string | null>(null);
	let decisionForm = $state<HTMLFormElement | null>(null);
	let discussion = $state<'comments' | 'activity'>('comments');
	let discussionTab = $state<string | undefined>();
	/** The stories whose criteria are unfolded under their rows; each opens and closes on its own. */
	const openStories = new SvelteSet<string>();
	/** Every story whose criteria editor has been opened: it stays mounted (hidden when folded), so an unsaved draft
	 *  survives folding the row and opening it again. */
	const mountedStories = new SvelteSet<string>();
	/** The New story form has a create in flight: its dialog cannot be dismissed until the answer is in. */
	let storyFormBusy = $state(false);

	let editingDetails = $state(false);
	let details = $state<EpicDetails>({ title: '', initiative: '' });
	/** Where the details editor started; its base is `{ version, text: fieldsText(detailsFrom) }` (#lib/editorBase.ts). */
	let detailsFrom: EpicDetails = { title: '', initiative: '' };
	let detailsBase: EditorBase = { version: 0, text: '' };
	/** A refusal or validation error of the details form; the page notice stays for other actions and for the reload text. */
	let detailsError = $state<string | null>(null);
	let detailsToggle = $state<HTMLButtonElement | null>(null);
	let detailsForm = $state<HTMLFormElement | null>(null);

	let decision = $state({ title: '', decider: '', due: '' });
	let decisionId = $state(newDraftKey());

	const blockingDecisions = $derived(blocking(decisions));
	const blockingIds = $derived(new Set(blockingDecisions.map((d) => d.id)));
	const acceptedCount = $derived(decisions.filter((d) => !d.closed && !blockingIds.has(d.id)).length);
	const showAccepted = $derived(accepted ?? (blockingDecisions.length === 0 && acceptedCount > 0));
	const closedCount = $derived(decisions.filter((d) => d.closed).length);
	const shownDecisions = $derived(decisions.filter((d) => showClosed || (showAccepted && acceptedCount === 0) || !d.closed));
	// Counted under the key the server returned: the URL may name the epic by an alias (an external reference).
	const totals = $derived(epic ? sumCounts(epicCounts(stories, [epic.key]).values()) : sumCounts([]));
	const initiativeTitle = $derived(initiatives.find((i) => i.key === epic?.initiative)?.title ?? null);
	const cell = $derived(
		blockerCell({
			blockingDecisions: epic ? epicStripDecisions(decisions, initiativeDecisions, epic, initiativeTitle) : [],
			storiesNeedingCriteria: totals.needCriteria,
			criteriaAnchor: '#stories'
		})
	);
	/** The promise dialog, with the sprints it offers shortcuts from (fetched each time it opens; until they answer the
	 *  dialog has no shortcuts and no `min`, and says so if they fail). The writes go through `write`. */
	const promise = new PromiseControl({
		path: 'epics',
		key: () => epic?.key,
		write: (action) => write(action),
		takeNotice: () => {
			const n = notice;
			notice = null;
			return n;
		},
		refocus: () => refocusActions(),
		fetchSprints: true
	});

	const menuItems = $derived<MenuItem[]>([
		{ label: 'Edit brief', onselect: startEdit, disabled: editing },
		...(epic?.discovery_complete ? [{ label: 'Clear discovery', onselect: () => void discovery(false), disabled: busy }] : []),
		{ label: 'Mark urgent…', onselect: openMark, disabled: busy },
		...(epic?.urgent_mark ? [{ label: 'Clear urgent mark', onselect: () => void clearUrgent(), disabled: busy }] : []),
		// An epic has no done state: the promise items depend only on whether it has a promise; its clear asks first.
		...promise.items(epic?.promise != null, false, busy)
	]);

	/** When the page last loaded: the urgent line's "N d ago" counts from it. */
	let loadedAt = $state(new Date());
	/** Whether the "Mark urgent" dialog is open, and the reason a failed mark left in it. */
	let marking = $state(false);
	let markError = $state<string | null>(null);
	/** The source's title, when the page has loaded it; otherwise the line shows its key and type only. */
	const titleOf = (r: ItemRef): string | undefined =>
		r.item === 'initiative' ? initiatives.find((i) => i.key === r.key)?.title : stories.find((s) => s.story.key === r.key)?.story.title;

	function openMark() {
		markError = null;
		marking = true;
	}

	async function markUrgent(reason: string | null) {
		const k = epic?.key;
		if (!k) return;
		const ok = await write(() =>
			unwrap(api.POST('/api/v1/epics/{key}/urgent', { params: { path: { key: k } }, body: { reason } }))
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
		const k = epic?.key;
		if (!k) return;
		if (await write(() => unwrap(api.DELETE('/api/v1/epics/{key}/urgent', { params: { path: { key: k } } })))) {
			await refocusActions();
		} else {
			// The Clear button that was pressed is still there: keep the keyboard on it, unless the user moved it.
			await tick();
			if (focusIsFree(null)) document.body.querySelector<HTMLElement>(`button[aria-label="Clear urgent mark on ${k}"]`)?.focus();
		}
	}

	/** The control that opened a write (the expired line's buttons, a dialog) is gone: focus goes to the Actions menu, unless the user moved it. */
	async function refocusActions() {
		await tick();
		if (focusIsFree(null)) document.body.querySelector<HTMLElement>(`button[aria-label="Actions for ${epic?.key}"]`)?.focus();
	}

	let latest = 0;

	/** Every write is followed by this, so each `version` is the one the server last returned. */
	async function reload() {
		const k = key;
		const seq = ++latest;
		try {
			const epicRead = unwrap(api.GET('/api/v1/epics/{key}', { params: { path: { key: k } } }));
			const [e, s, d, inis, id] = await Promise.all([
				epicRead,
				unwrap(api.GET('/api/v1/stories', { params: { query: { epic: k } } })),
				unwrap(api.GET('/api/v1/decisions', { params: { query: { on: k } } })),
				unwrap(api.GET('/api/v1/initiatives')),
				// A decision on the initiative holds up this epic too, so the strip counts it: read once the parent is known.
				epicRead.then((x) =>
					x.initiative ? unwrap(api.GET('/api/v1/decisions', { params: { query: { initiative: x.initiative } } })) : []
				)
			]);
			if (seq !== latest) return;
			epic = e;
			loadedAt = new Date();
			if (editing) briefBase = followBase(briefBase, { version: e.version, text: e.brief ?? '' });
			if (editingDetails) detailsBase = followBase(detailsBase, { version: e.version, text: fieldsText(epicDetails(e)) });
			stories = s;
			decisions = d;
			initiativeDecisions = id;
			initiatives = inis;
			loadError = null;
			staleNotice = null;
			// Advisory reads: each fails to an empty list rather than failing the page.
			const [f, t, a] = await Promise.all([
				unwrap(api.GET('/api/v1/epics/{key}/findings', { params: { path: { key: k } } })).catch(() => []),
				unwrap(api.GET('/api/v1/items/{key}/comments', { params: { path: { key: k } } })).catch(() => []),
				unwrap(api.GET('/api/v1/activity', { params: { query: { entity_type: 'epic', entity_key: e.key } } })).catch(() => [])
			]);
			if (seq !== latest) return;
			findings = f;
			comments = t;
			activity = a;
		} catch (e) {
			if (seq !== latest) return;
			if (epic) staleNotice = `Could not reload ${k}: ${errorMessage(e)}`;
			else loadError = errorMessage(e);
		}
	}

	$effect(() => {
		void key;
		editing = false;
		editingDetails = false;
		detailsError = null;
		notice = null;
		staleNotice = null;
		loadError = null;
		findings = [];
		comments = [];
		activity = [];
		stories = [];
		decisions = [];
		initiativeDecisions = [];
		epic = null;
		showClosed = false;
		accepted = null;
		creatingStory = null;
		addingDecision = null;
		decisionError = null;
		promise.reset();
		discussion = 'comments';
		openStories.clear();
		mountedStories.clear();
		decision = { title: '', decider: '', due: '' };
		decisionId = newDraftKey();
		reload();
	});
	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
	onLive(reload);
	/** A scheduled check that confirms its result logs nothing, so no live event arrives: the page refetches every two
	 *  minutes while visible as long as an open decision on the epic, its stories or its initiative has a record (the
	 *  list does not poll). */
	everyVisible(() => {
		if (epic && pollsChecks(decisions, initiativeDecisions, epic, stories)) void reload();
	}, 2 * 60_000);

	/** Refusals show their reasons; a conflict reloads (drafts are kept); anything else shows its message. */
	async function failed(e: unknown) {
		notice = reasonOf(e);
		if (e instanceof Conflict) await reload();
	}

	/** What a failed write tells the user. */
	function reasonOf(e: unknown): string {
		if (e instanceof Refused) return e.reasons.map((r) => fullLabel(r)).join('; ');
		if (e instanceof Conflict) return 'Someone else changed this; it was reloaded. Check it and try again.';
		if (e instanceof ApiError) return e.message;
		return errorMessage(e);
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

	function patchEpic(body: Omit<EditItem, 'version'>, version?: number) {
		const e = epic;
		if (!e) return Promise.resolve(false);
		return write(() =>
			unwrap(api.PATCH('/api/v1/epics/{key}', { params: { path: { key: e.key } }, body: { version: version ?? e.version, ...body } }))
		);
	}

	function startEdit() {
		briefUploads.reset();
		briefDraft = epic?.brief ?? '';
		briefBase = { version: epic?.version ?? 0, text: epic?.brief ?? '' };
		editing = true;
	}

	async function saveBrief(e: SubmitEvent) {
		e.preventDefault();
		if (briefUploads.busy) return;
		if (await patchEpic({ brief: briefDraft, ...briefUploads.body() }, briefBase.version)) {
			briefUploads.reset();
			editing = false;
		}
		// Only a conflict (which reloaded and told the user) rebases on what is loaded now, so trying again can succeed. Any
		// other failure keeps the base: a retry must not overwrite a colleague's text the user has not been shown.
		else if (conflicted && epic) briefBase = rebase({ version: epic.version, text: epic.brief ?? '' });
	}

	function startDetails(first = 'input') {
		const e = epic;
		if (!e) return;
		details = epicDetails(e);
		detailsFrom = epicDetails(e);
		detailsBase = { version: e.version, text: fieldsText(detailsFrom) };
		detailsError = null;
		editingDetails = true;
		// The trigger is replaced by the form: focus moves to its first field, not to <body>.
		void tick().then(() => detailsForm?.querySelector<HTMLElement>(first)?.focus());
	}

	/** Cancel, an unchanged save and a successful one all close the editor and return focus to its toggle. */
	function closeDetails() {
		// Only if focus is still in the form or on the page: a slow save must not take it from where the user went.
		const free = focusIsFree(detailsForm);
		editingDetails = false;
		detailsError = null;
		if (free) void tick().then(() => detailsToggle?.focus());
	}

	/** A failed save leaves the form open with its alert; its fields were disabled while saving, so focus goes back to the
	 *  first one, unless the user moved it elsewhere. */
	function refocusDetails() {
		void tick().then(() => {
			if (focusIsFree(detailsForm)) detailsForm?.querySelector('input')?.focus();
		});
	}

	async function saveDetails(ev: SubmitEvent) {
		ev.preventDefault();
		const out = epicEdit(detailsFrom, details);
		if (out.kind === 'unchanged') {
			closeDetails();
			return;
		}
		if (out.kind === 'invalid') {
			detailsError = out.error;
			return;
		}
		detailsError = null;
		if (await patchEpic(out.body, detailsBase.version)) closeDetails();
		else if (!conflicted) {
			// Not a conflict: the reason belongs to the form, not to the page notice.
			detailsError = notice;
			notice = null;
		} else if (epic) {
			// The user's changed fields stay; every other field takes what was just reloaded.
			const server = epicDetails(epic);
			details = mergeDraft(EPIC_SAME, detailsFrom, details, server);
			detailsFrom = server;
			detailsBase = rebase({ version: epic.version, text: fieldsText(server) });
		}
		if (editingDetails) refocusDetails();
	}

	async function discovery(complete: boolean) {
		const e = epic;
		if (!e) return;
		if (!complete && !confirm(`Clear "discovery complete" on ${e.key}? The change is logged.`)) return;
		await write(() =>
			unwrap(
				api.POST('/api/v1/epics/{key}/discovery', {
					params: { path: { key: e.key } },
					body: { version: e.version, complete }
				})
			)
		);
	}

	/** Sends the open dialog's draft. The opening is read when the form submits, so its answer acts on that dialog only: a
	 *  dialog opened since (on this page or after a page change) keeps its draft, its alert and its busy state. */
	async function requireDecision(ev: SubmitEvent) {
		ev.preventDefault();
		const opening = addingDecision;
		if (opening === null || sendingDecision === opening) return;
		const d = { ...decision };
		if (!d.title.trim() || !d.decider.trim()) return;
		const on = key;
		decisionError = null;
		sendingDecision = opening;
		let ok = true;
		let error: unknown;
		try {
			await unwrap(
				api.POST('/api/v1/decisions', {
					body: {
						idempotency_key: draftKey(decisionId, JSON.stringify([on, d.title.trim(), d.decider.trim(), d.due])),
						on,
						title: d.title.trim(),
						decider: d.decider.trim(),
						due: d.due || null
					}
				})
			);
		} catch (e) {
			ok = false;
			error = e;
		}
		if (sendingDecision === opening) sendingDecision = null;
		const current = addingDecision === opening;
		if (ok) {
			// Only if the form still holds what was sent (its inputs are disabled while the create is pending, so this is a guard).
			if (current && decision.title === d.title && decision.decider === d.decider && decision.due === d.due) {
				decision = { title: '', decider: '', due: '' };
				decisionId = newDraftKey();
			}
			if (current) addingDecision = null;
			await reload();
			return;
		}
		if (!current) {
			// The dialog it was sent from is gone (the page changed): the reason goes to the page, naming the item.
			notice = `Could not add the decision on ${on}: ${reasonOf(error)}`;
			if (error instanceof Conflict) await reload();
			return;
		}
		// The page notice would sit behind the modal: the reason belongs to the form.
		decisionError = reasonOf(error);
		if (error instanceof Conflict) await reload();
		// Its fields were disabled while sending, so focus fell to the page: back to the first, unless the user moved it.
		void tick().then(() => {
			if (addingDecision === opening && focusIsFree(decisionForm)) decisionForm?.querySelector('input')?.focus();
		});
	}

	/** The create callback of the "New story" dialog opened as `opening`: closes that dialog only if it is still the open one.
	 *  The token is a parameter: the form reads the prop when it submits, while its dialog is open, and the closure keeps that
	 *  number. A `{@const}` token would be read lazily in a production build, first when the answer arrives, and so hold
	 *  the newer opening. */
	function storyCreated(opening: number): () => Promise<void> {
		return async () => {
			if (creatingStory === opening) creatingStory = null;
			await reload();
		};
	}

	function closeAddDecision() {
		addingDecision = null;
		decisionError = null;
	}

	/** Unfolds the decisions that block nothing; with no accepted ones, the fold is the closed ones alone (shownDecisions). */
	function toggleRest() {
		accepted = !showAccepted;
	}

	function toggleStory(k: string) {
		if (openStories.has(k)) openStories.delete(k);
		else {
			openStories.add(k);
			mountedStories.add(k);
		}
	}

	/** Sends the whole list as the editor shows it, each criterion with its own `done`. */
	async function saveCriteria(s: Story, criteria: Story['criteria']) {
		await write(() =>
			unwrap(
				api.PATCH('/api/v1/stories/{key}', {
					params: { path: { key: s.key } },
					body: { version: s.version, criteria }
				})
			)
		);
	}
</script>

<svelte:head><title>{epic?.title ?? key} · Reeve</title></svelte:head>

{#if loadError}
	<div class="page">
		<Notice tone="error" onretry={reload}>Could not load {key}: {loadError}</Notice>
	</div>
{:else if !epic}
	<!-- The first load keeps the page's frame and says it is loading. -->
	<div class="page" aria-busy="true">
		<nav aria-label="Breadcrumb" class="crumb"><a href="/planning">Planning</a></nav>
		<p class="muted">Loading…</p>
	</div>
{:else}
	{@const ep = epic}
	<div class="page">
		{#if staleNotice}
			<Notice onretry={reload} ondismiss={() => (staleNotice = null)}>{staleNotice}</Notice>
		{/if}

		<header class="head">
			<nav aria-label="Breadcrumb" class="crumb">
				<a href="/planning">Planning</a>
				{#if ep.initiative}
					/ <a href="/initiatives/{encodeURIComponent(ep.initiative)}">{ep.initiative}</a>
				{/if}
			</nav>
			<div class="title-row">
				<h1>{ep.title}</h1>
				<GoalLinkButton key={ep.key} />
				{#if !editingDetails}
					<button type="button" bind:this={detailsToggle} onclick={() => startDetails()}>Edit details</button>
				{/if}
				<ActionsMenu label="Actions for {ep.key}" items={menuItems} />
			</div>
			<p class="meta">
				<span class="key"><Glyph level="epic" />{ep.key}</span>
				{#if initiativeTitle}<span>in {initiativeTitle}</span>{/if}
				<span>{stories.length} {stories.length === 1 ? 'story' : 'stories'}</span>
				{#if ep.external_ref}<span title="External reference">from {ep.external_ref}</span>{/if}
			</p>
			<UrgentLine
				itemKey={ep.key}
				mark={ep.urgent_mark}
				sources={ep.urgent_sources}
				{titleOf}
				now={loadedAt}
				{busy}
				onremark={openMark}
				onclear={() => void clearUrgent()}
			/>
			{#if ep.promise}<PromiseLine promise={ep.promise} />{/if}
			{#if editingDetails}
				<form bind:this={detailsForm} class="require" aria-label="Epic details" onsubmit={saveDetails}>
					<label>Title <input bind:value={details.title} required disabled={busy} /></label>
					<label>
						Initiative
						<Picker label="Initiative" options={keyed(orderedInitiatives)} bind:value={details.initiative} empty="none" disabled={busy} />
					</label>
					{#if detailsError}<p class="error" role="alert">{detailsError}</p>{/if}
					<button type="button" onclick={closeDetails}>Cancel</button>
					<button type="submit" disabled={busy}>Save details</button>
				</form>
			{/if}
		</header>

		{#if !ep.initiative}
			<p class="banner" role="status">
				<span>This epic belongs to no initiative.</span>
				{#if !editingDetails}<button type="button" onclick={() => startDetails('input[role="combobox"]')}>Choose an initiative</button>{/if}
			</p>
		{/if}

		{#if notice}
			<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>
		{/if}

		<StatusStrip
			discovery={ep.discovery_complete
				? {
						done: true,
						by: short(ep.discovery_complete.by),
						at: new Date(ep.discovery_complete.at).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
					}
				: { done: false }}
			{cell}
			progress={{ done: totals.done, total: totals.total }}
			onmarkdiscovery={() => void discovery(true)}
		/>

		<Findings {findings} />

		<section aria-labelledby="brief">
			<h2 id="brief">Brief</h2>
			{#if editing}
				<form class="brief-edit" onsubmit={saveBrief}>
					<textarea
						bind:this={briefField}
						bind:value={briefDraft}
						disabled={busy}
						rows="12"
						aria-label="brief (Markdown)"
						{@attach attachable(briefUploads)}
					></textarea>
					<ImageUploadsRow uploads={briefUploads} field={briefField} disabled={busy} />
					<div class="actions">
						<button
							type="button"
							onclick={() => {
								editing = false;
								briefUploads.reset();
							}}>Cancel</button
						>
						<button type="submit" disabled={busy || briefUploads.busy}>{briefUploads.busy ? 'Uploading…' : 'Save brief'}</button>
					</div>
				</form>
			{:else}
				<Brief markdown={ep.brief} />
			{/if}
			<Attachments itemKey={ep.key} />
		</section>

		<section id="stories" aria-labelledby="stories-h">
			<div class="section-head">
				<h2 id="stories-h">Stories <span class="count">{stories.length}</span></h2>
				<button
					type="button"
					onclick={() => {
						storyFormBusy = false;
						creatingStory = ++storyOpenings;
					}}>New story</button
				>
			</div>
			{#if stories.length === 0}
				<p class="muted">No stories yet</p>
			{:else}
				<ul class="rows stories">
					{#each stories as item (item.story.key)}
						{@const s = item.story}
						{@const st = storyStatus(item)}
						{@const note = freshNote(live.recent, s.key, session.me?.email)}
						{@const expanded = openStories.has(s.key)}
						<li class:fresh={note} class:done={s.delivery === 'done'}>
							<div class="row">
								<button
									type="button"
									class="ghost icon chevron"
									class:open={expanded}
									aria-expanded={expanded}
									aria-controls={mountedStories.has(s.key) ? `criteria-${s.key}` : undefined}
									aria-label="Criteria for {s.key}"
									onclick={() => toggleStory(s.key)}><span aria-hidden="true">›</span></button
								>
								<span class="key"><Glyph level={levelOfKind(s.kind)} />{s.key}</span>
								{#if s.kind !== 'story'}<span class="kind">{s.kind === 'bug' ? 'Bug' : 'Chore'}</span>{/if}
								<a class="title" data-sveltekit-reset="false" href={storyHref(page.url, s.key)}>{s.title}</a>
								<!-- Every row is under this epic: naming it again (" · via KEY") would say nothing. -->
								{#if s.promise}<PromiseChip promise={s.promise} via={false} />{/if}
								<span
									class="readiness"
									title={item.readiness.ready ? undefined : item.readiness.reasons.map((r) => fullLabel(r)).join('\n')}
									><StatusDot tone={st.tone} label={st.word} /></span
								>
								<span class="column">{COLUMN_LABEL[s.delivery]}</span>
								<Avatar email={s.owner} />
							</div>
							{#if note}<p class="note">{note}</p>{/if}
							{#if mountedStories.has(s.key)}
								<div id="criteria-{s.key}" class="criteria" hidden={!expanded}>
									<CriteriaEditor
										storyKey={s.key}
										criteria={s.criteria}
										{busy}
										onsave={(criteria) => saveCriteria(s, criteria)}
									/>
								</div>
							{/if}
						</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section id="decisions" aria-labelledby="decisions-h">
			<div class="section-head">
				<!-- The separators sit inside template literals: Svelte trims a span's leading space, which would run "Decisions" into the count. -->
				<h2 id="decisions-h">Decisions{#if blockingDecisions.length}<span class="count">{` · ${blockingDecisions.length} blocking`}</span>{/if}{#if acceptedCount}<span class="count">{` · ${acceptedCount} accepted`}</span>{/if}</h2>
				<button type="button" onclick={() => (addingDecision = ++decisionOpenings)}>+ Add decision</button>
			</div>
			{#key key}
				<!-- One list over both groups: a decision that moves between them keeps its editor, draft and thread. -->
				<DecisionList
					decisions={shownDecisions}
					decisionHref={(id) => decisionHref(page.url, id)}
					onchanged={reload}
					poll={false}
					rest={(d) => !blockingIds.has(d.id)}
					showRest={showAccepted}
					restId="decisions-rest"
					empty="Nothing blocking"
				>
					{#snippet between()}
						{#if acceptedCount > 0 || closedCount > 0}
							<div class="fold">
								<button
									type="button"
									class="ghost"
									aria-expanded={showAccepted}
									aria-controls={showAccepted ? 'decisions-rest' : undefined}
									onclick={toggleRest}
									>{acceptedCount === 0
										? `Show closed (${closedCount})`
										: `${acceptedCount} accepted ${acceptedCount === 1 ? 'decision' : 'decisions'}`}</button
								>
								{#if showAccepted && acceptedCount > 0 && closedCount > 0}
									<label class="toggle"><input type="checkbox" bind:checked={showClosed} /> show closed ({closedCount})</label>
								{/if}
							</div>
						{/if}
					{/snippet}
				</DecisionList>
			{/key}
		</section>

		<section aria-labelledby="discussion-h">
			<h2 id="discussion-h">Discussion</h2>
			<div class="discussion">
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
					panel="epic-discussion-panel"
					bind:selectedTabId={discussionTab}
				/>
				<div role="tabpanel" tabindex="0" id="epic-discussion-panel" class="tabpanel" aria-labelledby={discussionTab}>
					<!-- Comments stay mounted, so a half-written comment survives a switch to Activity and back. The activity list is
					     rendered only while shown: its owner emails in title attributes would otherwise repeat the page's. -->
					<div hidden={discussion !== 'comments'}>
						<Comments
							itemKey={ep.key}
							{comments}
							onposted={(t) => {
								// A reload started before the post could answer with the thread before it; reloading now supersedes it.
								comments = t;
								reload();
							}}
						/>
					</div>
					{#if discussion === 'activity'}<ActivityList entries={activity} />{/if}
				</div>
			</div>
		</section>
	</div>

	{#if addingDecision !== null}
		<Dialog title="Add decision" onclose={closeAddDecision} closable={!decisionBusy}>
			<form bind:this={decisionForm} class="decision-form" aria-label="Add decision" onsubmit={requireDecision}>
				<label class="field">Title <input bind:value={decision.title} required disabled={decisionBusy} /></label>
				<label class="field">Decider <input type="email" bind:value={decision.decider} required disabled={decisionBusy} placeholder="name@example.com" /></label>
				<label class="field">Due <input type="date" bind:value={decision.due} disabled={decisionBusy} /></label>
				<DueShortcuts bind:value={decision.due} disabled={decisionBusy} />
				{#if decisionError}<p class="error" role="alert">{decisionError}</p>{/if}
				<button type="submit" class="primary" disabled={decisionBusy || !decision.title.trim() || !decision.decider.trim()}>Require decision</button>
			</form>
		</Dialog>
	{/if}

	{#if marking}
		<MarkUrgentDialog
			item={ep.key}
			initial={ep.urgent_mark?.reason ?? null}
			{busy}
			error={markError}
			onsubmit={markUrgent}
			oncancel={() => {
				if (!busy) marking = false;
			}}
		/>
	{/if}

	{#if promise.open}
		<PromiseDialog
			key={ep.key}
			activeEnds={promise.sprints?.active?.sprint.ends_on ?? null}
			draftEnds={promise.sprints?.draft?.ends_on ?? null}
			today={promise.sprints?.today ?? null}
			sprintsFailed={promise.sprintsFailed}
			current={ep.promise?.fact ?? null}
			{busy}
			error={promise.error}
			onsave={(body) => void promise.save(body)}
			onclose={() => promise.close(busy)}
		/>
	{/if}

	{#if promise.confirming}
		<ClearPromiseDialog
			item={ep.key}
			kind="epic"
			{busy}
			error={promise.confirmError}
			onconfirm={() => void promise.clear()}
			oncancel={() => promise.cancelClear(busy)}
		/>
	{/if}

	{#if creatingStory !== null}
		<Dialog title="New story" onclose={() => (creatingStory = null)} closable={!storyFormBusy}>
			<!-- No {#key}: the whole page body, this dialog included, unmounts while another epic loads. -->
			<StoryForm
				epic={epic.key}
				disabled={busy}
				bind:busy={storyFormBusy}
				oncreated={storyCreated(creatingStory)}
			/>
		</Dialog>
	{/if}
{/if}

<style>
	.page {
		display: grid;
		gap: 28px;
		max-width: 980px;
		margin: 0 auto;
	}
	.head {
		display: grid;
		gap: 6px;
	}
	.crumb,
	.meta {
		margin: 0;
		color: var(--muted);
		font-size: 14px;
	}
	.title-row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 12px;
	}
	.title-row h1 {
		flex: 1;
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.meta {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 12px;
	}
	.banner {
		display: flex;
		flex-wrap: wrap;
		gap: 8px 12px;
		align-items: center;
		margin: 0;
		padding: 8px 12px;
		border: 1px solid var(--warn);
		border-radius: 6px;
		background: var(--raised);
	}
	section {
		display: grid;
		gap: 10px;
		min-width: 0;
	}
	.section-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}
	/* The shared .rows list (app.css) gives the frame and the live-change ring; each row stays a grid, so the criteria
	 * unfold under it, and nothing is clipped: the corners are rounded per row instead. */
	.stories {
		overflow: visible;
	}
	.stories > li {
		display: grid;
		align-items: normal;
		gap: 6px;
		padding: 6px 10px;
		border-top: 1px solid var(--line);
	}
	.stories > li:first-child {
		border-top: 0;
		border-radius: 6px 6px 0 0;
	}
	.stories > li:last-child {
		border-radius: 0 0 6px 6px;
	}
	.row {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 12px;
		min-width: 0;
	}
	.chevron span {
		display: inline-block;
		transition: transform 0.1s;
	}
	.chevron.open span {
		transform: rotate(90deg);
	}
	.key {
		color: var(--muted);
	}
	.title {
		flex: 1 1 200px;
		min-width: 0;
		overflow-wrap: anywhere;
		color: var(--foreground);
		font-weight: 500;
		text-decoration: none;
	}
	.title:hover {
		text-decoration: underline;
	}
	.done .title {
		color: var(--muted);
	}
	.readiness,
	.column {
		font-size: 13px;
	}
	.column {
		color: var(--muted);
	}
	.kind {
		font-size: 13px;
		font-weight: 600;
	}
	.criteria {
		padding: 4px 0 6px 36px;
	}
	.fold {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 8px 16px;
	}
	.discussion {
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.tabpanel {
		padding: 12px;
	}
	.brief-edit {
		display: grid;
		gap: 0.5rem;
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
	.toggle {
		display: block;
		font-size: 14px;
	}
	.require {
		display: flex;
		flex-wrap: wrap;
		gap: 0.5rem 1rem;
		align-items: end;
		padding: 0.75rem;
		border-radius: 6px;
		background: var(--lighter-background);
	}
	.require label {
		display: grid;
		gap: 0.2rem;
		font-size: 14px;
	}
	.decision-form {
		display: grid;
		gap: 12px;
	}
	.decision-form button {
		justify-self: start;
	}
	.error {
		margin: 0;
		flex-basis: 100%;
		color: var(--danger);
	}
</style>
