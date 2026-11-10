<script lang="ts">
	import Glyph from '#lib/components/Glyph.svelte';
	import GoalLinkButton from '#lib/components/GoalLinkButton.svelte';
	import { page } from '$app/state';
	import { api, ApiError, Conflict, Refused, unwrap } from '#lib/api/client.ts';
	import type { ItemRef, ActivityEntry, Epic, Initiative, RequiredDecision, Finding, Comment, StoryWithReadiness } from '#lib/api/types.ts';
	import Findings from '#lib/components/Findings.svelte';
	import Comments from '#lib/components/Comments.svelte';
	import { fullLabel } from '#lib/board.ts';
	import Brief from '#lib/components/Brief.svelte';
	import DecisionList from '#lib/components/DecisionList.svelte';
	import { blocking, itemHref } from '#lib/decisions.ts';
	import ActivityList from '#lib/components/ActivityList.svelte';
	import ActionsMenu, { type MenuItem } from '#lib/ui/ActionsMenu.svelte';
	import Dialog from '#lib/ui/Dialog.svelte';
	import MarkUrgentDialog from '#lib/components/MarkUrgentDialog.svelte';
	import UrgentLine from '#lib/components/UrgentLine.svelte';
	import Notice from '#lib/ui/Notice.svelte';
	import StatusStrip from '#lib/ui/StatusStrip.svelte';
	import Toggle from '#lib/ui/Toggle.svelte';
	import { blockerCell } from '#lib/strip.ts';
	import { epicCounts, stripDecisions, sumCounts, waitingOn } from '#lib/initiative.ts';
	import EpicCard from '#lib/components/EpicCard.svelte';
	import { everyVisible, onLive } from '#lib/events.svelte.ts';
	import { errorMessage, month, short } from '#lib/format.ts';
	import { draftKey, newDraftKey } from '#lib/idempotency.ts';
	import { followBase, rebase, type EditorBase } from '#lib/editorBase.ts';
	import NewEpicForm from '#lib/components/NewEpicForm.svelte';
	import { INITIATIVE_SAME, fieldsText, initiativeDetails, initiativeEdit, mergeDraft, type InitiativeDetails } from '#lib/forms.ts';
	import { tick } from 'svelte';
	import { focusIsFree } from '#lib/focus.ts';

	const key = $derived(page.params.key ?? '');

	let initiative = $state<Initiative | null>(null);
	let epics = $state<Epic[]>([]);
	let decisions = $state<RequiredDecision[]>([]);
	/** Advisory, so fetched best effort: a failed read shows none rather than failing the page. */
	let findings = $state<Finding[]>([]);
	/** The item's comment thread, fetched best effort like the findings. */
	let comments = $state<Comment[]>([]);
	/** The initiative's activity log, fetched best effort like the comments. */
	let activity = $state<ActivityEntry[]>([]);
	/** The stories of this initiative's epics: what the epic cards and the status strip count. */
	let stories = $state<StoryWithReadiness[]>([]);
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
	let showClosed = $state(false);
	/** Whether the decisions that block nothing (accepted, and closed ones when shown) are unfolded. */
	/** The user's choice for the accepted fold; null follows the default (open when nothing blocks). */
	let accepted = $state<boolean | null>(null);
	/** The open "New epic" dialog's opening, or null: a create's answer closes only the dialog it was sent from. */
	let creatingEpic = $state<number | null>(null);
	let epicOpenings = 0;
	/** The New epic form has a create in flight: its dialog cannot be dismissed until the answer is in. */
	let epicFormBusy = $state(false);
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

	let editingDetails = $state(false);
	let details = $state<InitiativeDetails>({ title: '', target_month: '', owner: '' });
	/** Where the details editor started; its base is `{ version, text: fieldsText(detailsFrom) }` (#lib/editorBase.ts). */
	let detailsFrom: InitiativeDetails = { title: '', target_month: '', owner: '' };
	let detailsBase: EditorBase = { version: 0, text: '' };
	/** A refusal or validation error of the details form; the page notice stays for other actions and for the reload text. */
	let detailsError = $state<string | null>(null);
	let detailsToggle = $state<HTMLButtonElement | null>(null);
	let detailsForm = $state<HTMLFormElement | null>(null);

	// The "require a decision" form; its idempotency key follows the text sent (see #lib/idempotency.ts).
	let decision = $state({ title: '', decider: '', due: '' });
	let decisionId = $state(newDraftKey());

	const epicKeys = $derived(new Set(epics.map((e) => e.key)));
	const counts = $derived(epicCounts(stories, epicKeys));
	const totals = $derived(sumCounts(counts.values()));
	// Matched against the key the server returned: the URL may name the initiative by an alias (an external reference).
	const waits = $derived(waitingOn(decisions, initiative?.key ?? key));
	const blockingDecisions = $derived(blocking(decisions));
	const blockingIds = $derived(new Set(blockingDecisions.map((d) => d.id)));
	const acceptedCount = $derived(decisions.filter((d) => !d.closed && !blockingIds.has(d.id)).length);
	const showAccepted = $derived(accepted ?? (blockingDecisions.length === 0 && acceptedCount > 0));
	const closedCount = $derived(decisions.filter((d) => d.closed).length);
	/** Closed decisions are listed only when asked for: by the checkbox, or by unfolding a fold that holds nothing else. */
	const shownDecisions = $derived(decisions.filter((d) => showClosed || (showAccepted && acceptedCount === 0) || !d.closed));
	const cell = $derived(
		blockerCell({
			blockingDecisions: initiative ? stripDecisions(decisions, initiative, epics) : [],
			storiesNeedingCriteria: totals.needCriteria,
			criteriaAnchor: '#epics'
		})
	);
	const menuItems = $derived<MenuItem[]>([
		{ label: 'Edit brief', onselect: startEdit, disabled: editing },
		...(initiative?.discovery_complete ? [{ label: 'Clear discovery', onselect: () => void discovery(false), disabled: busy }] : []),
		{ label: 'Mark urgent…', onselect: openMark, disabled: busy },
		...(initiative?.urgent_mark ? [{ label: 'Clear urgent mark', onselect: () => void clearUrgent(), disabled: busy }] : [])
	]);

	/** When the page last loaded: the urgent line's "N d ago" counts from it. */
	let loadedAt = $state(new Date());
	/** Whether the "Mark urgent" dialog is open, and the reason a failed mark left in it. */
	let marking = $state(false);
	let markError = $state<string | null>(null);
	/** The source's title, when the page has loaded it; otherwise the line shows its key and type only. */
	const titleOf = (r: ItemRef): string | undefined =>
		r.item === 'epic'
			? epics.find((e) => e.key === r.key)?.title
			: r.item === 'story'
				? stories.find((s) => s.story.key === r.key)?.story.title
				: undefined;

	function openMark() {
		markError = null;
		marking = true;
	}

	async function markUrgent(reason: string | null) {
		const k = initiative?.key;
		if (!k) return;
		const ok = await write(() =>
			unwrap(api.POST('/api/v1/initiatives/{key}/urgent', { params: { path: { key: k } }, body: { reason } }))
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
		const k = initiative?.key;
		if (!k) return;
		if (await write(() => unwrap(api.DELETE('/api/v1/initiatives/{key}/urgent', { params: { path: { key: k } } })))) {
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
		if (focusIsFree(null)) document.body.querySelector<HTMLElement>(`button[aria-label="Actions for ${initiative?.key}"]`)?.focus();
	}

	let latest = 0;

	/** Every write is followed by this, so `version` is always the one the server last returned. */
	async function reload() {
		const k = key;
		const seq = ++latest;
		try {
			const [i, e, d, s] = await Promise.all([
				unwrap(api.GET('/api/v1/initiatives/{key}', { params: { path: { key: k } } })),
				unwrap(api.GET('/api/v1/epics', { params: { query: { initiative: k } } })),
				unwrap(api.GET('/api/v1/decisions', { params: { query: { initiative: k } } })),
				// GET /stories has no initiative filter: every story is read and those of this initiative's epics kept.
				unwrap(api.GET('/api/v1/stories'))
			]);
			if (seq !== latest) return;
			initiative = i;
			loadedAt = new Date();
			if (editing) briefBase = followBase(briefBase, { version: i.version, text: i.brief ?? '' });
			if (editingDetails) detailsBase = followBase(detailsBase, { version: i.version, text: fieldsText(initiativeDetails(i)) });
			epics = e;
			decisions = d;
			const keys = new Set(e.map((x) => x.key));
			stories = s.filter((x) => !!x.story.epic && keys.has(x.story.epic));
			loadError = null;
			staleNotice = null;
			// Advisory reads: each fails to an empty list rather than failing the page.
			const [f, t, a] = await Promise.all([
				unwrap(api.GET('/api/v1/initiatives/{key}/findings', { params: { path: { key: k } } })).catch(() => []),
				unwrap(api.GET('/api/v1/items/{key}/comments', { params: { path: { key: k } } })).catch(() => []),
				unwrap(api.GET('/api/v1/activity', { params: { query: { entity_type: 'initiative', entity_key: i.key } } })).catch(
					() => []
				)
			]);
			if (seq !== latest) return;
			findings = f;
			comments = t;
			activity = a;
		} catch (e) {
			if (seq !== latest) return;
			if (initiative) staleNotice = `Could not reload ${k}: ${errorMessage(e)}`;
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
		initiative = null;
		showClosed = false;
		accepted = null;
		creatingEpic = null;
		addingDecision = null;
		decisionError = null;
		discussion = 'comments';
		decision = { title: '', decider: '', due: '' };
		decisionId = newDraftKey();
		reload();
	});
	// Any activity entry, and every (re)connect, refetches: the dependency graph is the whole tree (#lib/events.svelte.ts).
	onLive(reload);
	/** A scheduled check that finds what the last one found logs nothing, so no live event comes: the page reloads every
	 *  two minutes while visible as long as an open decision has a record, which keeps every list's "checked" fresh (its
	 *  DecisionList does not poll on its own here). */
	everyVisible(() => {
		if (decisions.some((d) => d.record && !d.closed)) void reload();
	}, 2 * 60_000);

	/** Refusals show their reasons; a conflict reloads (an open brief draft is kept); anything else shows its message. */
	async function failed(e: unknown) {
		notice = reasonOf(e);
		if (e instanceof Conflict) await reload();
	}

	/** What a failed write tells the user. */
	function reasonOf(e: unknown): string {
		if (e instanceof Refused) return e.reasons.map((r) => fullLabel(r)).join('; ');
		if (e instanceof Conflict) return 'Someone else changed this initiative; it was reloaded. Check it and try again.';
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

	function startEdit() {
		briefDraft = initiative?.brief ?? '';
		briefBase = { version: initiative?.version ?? 0, text: initiative?.brief ?? '' };
		editing = true;
	}

	async function saveBrief(e: SubmitEvent) {
		e.preventDefault();
		const i = initiative;
		if (!i) return;
		const ok = await write(() =>
			unwrap(
				api.PATCH('/api/v1/initiatives/{key}', {
					params: { path: { key: i.key } },
					body: { version: briefBase.version, brief: briefDraft }
				})
			)
		);
		if (ok) editing = false;
		// Only a conflict (which reloaded and told the user) rebases on what is loaded now, so trying again can succeed. Any
		// other failure keeps the base: a retry must not overwrite a colleague's text the user has not been shown.
		else if (conflicted && initiative) briefBase = rebase({ version: initiative.version, text: initiative.brief ?? '' });
	}

	function startDetails() {
		const i = initiative;
		if (!i) return;
		details = initiativeDetails(i);
		detailsFrom = initiativeDetails(i);
		detailsBase = { version: i.version, text: fieldsText(detailsFrom) };
		detailsError = null;
		editingDetails = true;
		// The trigger is replaced by the form: focus moves to its first field, not to <body>.
		void tick().then(() => detailsForm?.querySelector('input')?.focus());
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

	async function saveDetails(e: SubmitEvent) {
		e.preventDefault();
		const i = initiative;
		if (!i) return;
		const out = initiativeEdit(detailsFrom, details);
		if (out.kind === 'unchanged') {
			closeDetails();
			return;
		}
		if (out.kind === 'invalid') {
			detailsError = out.error;
			return;
		}
		detailsError = null;
		const ok = await write(() =>
			unwrap(
				api.PATCH('/api/v1/initiatives/{key}', {
					params: { path: { key: i.key } },
					body: { version: detailsBase.version, ...out.body }
				})
			)
		);
		if (ok) closeDetails();
		else if (!conflicted) {
			// Not a conflict: the reason belongs to the form, not to the page notice.
			detailsError = notice;
			notice = null;
		} else if (initiative) {
			// The user's changed fields stay; every other field takes what was just reloaded.
			const server = initiativeDetails(initiative);
			details = mergeDraft(INITIATIVE_SAME, detailsFrom, details, server);
			detailsFrom = server;
			detailsBase = rebase({ version: initiative.version, text: fieldsText(server) });
		}
		if (editingDetails) refocusDetails();
	}

	async function discovery(complete: boolean) {
		const i = initiative;
		if (!i) return;
		if (!complete && !confirm(`Clear "discovery complete" on ${i.key}? The change is logged.`)) return;
		await write(() =>
			unwrap(
				api.POST('/api/v1/initiatives/{key}/discovery', {
					params: { path: { key: i.key } },
					body: { version: i.version, complete }
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

	/** The create callback of the "New epic" dialog opened as `opening`: closes that dialog only if it is still the open one.
	 *  The token is a parameter: the form reads the prop when it submits, while its dialog is open, and the closure keeps that
	 *  number. A `{@const}` token would be read lazily in a production build, first when the answer arrives, and so hold
	 *  the newer opening. */
	function epicCreated(opening: number): () => Promise<void> {
		return async () => {
			if (creatingEpic === opening) creatingEpic = null;
			await reload();
		};
	}

	function closeAddDecision() {
		addingDecision = null;
		decisionError = null;
	}

	/** Unfolds the decisions that block nothing; with no accepted ones, the fold is the closed ones alone (restDecisions). */
	function toggleRest() {
		accepted = !showAccepted;
	}
</script>

<svelte:head><title>{initiative?.title ?? key} · Reeve</title></svelte:head>

{#if loadError}
	<div class="page">
		<Notice tone="error" onretry={reload}>Could not load {key}: {loadError}</Notice>
	</div>
{:else if !initiative}
	<!-- The first load keeps the page's frame and says it is loading. -->
	<div class="page" aria-busy="true">
		<nav aria-label="Breadcrumb" class="crumb"><a href="/">Planning</a></nav>
		<p class="muted">Loading…</p>
	</div>
{:else}
	{@const i = initiative}
	<div class="page">
		{#if staleNotice}
			<Notice onretry={reload} ondismiss={() => (staleNotice = null)}>{staleNotice}</Notice>
		{/if}

		<header class="head">
			<nav aria-label="Breadcrumb" class="crumb"><a href="/">Planning</a> / {i.horizon}</nav>
			<div class="title-row">
				<h1>{i.title}</h1>
				<GoalLinkButton key={i.key} />
				{#if !editingDetails}
					<button type="button" bind:this={detailsToggle} onclick={startDetails}>Edit details</button>
				{/if}
				<ActionsMenu label="Actions for {i.key}" items={menuItems} />
			</div>
			<p class="meta">
				<span class="key"><Glyph level="initiative" />{i.key}</span>
				{#if i.owner}<span title={i.owner}>{short(i.owner)}</span>{:else}<span>Unowned</span>{/if}
				{#if i.target_month}<span>Target {month(i.target_month)}</span>{/if}
				{#if i.external_ref}<span title="External reference">from {i.external_ref}</span>{/if}
			</p>
			<UrgentLine
				itemKey={i.key}
				mark={i.urgent_mark}
				sources={i.urgent_sources}
				{titleOf}
				now={loadedAt}
				{busy}
				onremark={openMark}
				onclear={() => void clearUrgent()}
			/>
			{#if editingDetails}
				<form bind:this={detailsForm} class="require" aria-label="Initiative details" onsubmit={saveDetails}>
					<label>Title <input bind:value={details.title} required disabled={busy} /></label>
					<label>Target month <input bind:value={details.target_month} placeholder="YYYY-MM" disabled={busy} /></label>
					<label>Owner <input type="email" bind:value={details.owner} placeholder="unowned" disabled={busy} /></label>
					{#if detailsError}<p class="error" role="alert">{detailsError}</p>{/if}
					<button type="button" onclick={closeDetails}>Cancel</button>
					<button type="submit" disabled={busy}>Save details</button>
				</form>
			{/if}
		</header>

		{#if notice}
			<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>
		{/if}

		<StatusStrip
			discovery={i.discovery_complete
				? {
						done: true,
						by: short(i.discovery_complete.by),
						at: new Date(i.discovery_complete.at).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' })
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
					<textarea bind:value={briefDraft} rows="12" aria-label="brief (Markdown)"></textarea>
					<div class="actions">
						<button type="button" onclick={() => (editing = false)}>Cancel</button>
						<button type="submit" disabled={busy}>Save brief</button>
					</div>
				</form>
			{:else}
				<Brief markdown={i.brief} />
			{/if}
		</section>

		<section id="epics" aria-labelledby="epics-h">
			<div class="section-head">
				<h2 id="epics-h">Epics <span class="count">{epics.length}</span></h2>
				<button
					type="button"
					onclick={() => {
						epicFormBusy = false;
						creatingEpic = ++epicOpenings;
					}}>New epic</button
				>
			</div>
			{#if epics.length === 0}
				<p class="muted">No epics yet</p>
			{:else}
				<ul class="epic-grid">
					{#each epics as e (e.key)}
						<EpicCard epic={e} counts={counts.get(e.key) ?? sumCounts([])} waiting={waits(e.key)} />
					{/each}
				</ul>
			{/if}
		</section>

		<section id="decisions" aria-labelledby="decisions-h">
			<div class="section-head">
				<h2 id="decisions-h">Decisions{#if blockingDecisions.length}<span class="count">{` · ${blockingDecisions.length} blocking`}</span>{/if}{#if acceptedCount}<span class="count">{` · ${acceptedCount} accepted`}</span>{/if}</h2>
				<button type="button" onclick={() => (addingDecision = ++decisionOpenings)}>+ Add decision</button>
			</div>
			<p class="muted">On this initiative, its epics and their stories; each lives on the item it gates.</p>
			{#key key}
				<!-- One list over both groups: a decision that moves between them keeps its editor, draft and thread. -->
				<DecisionList
					decisions={shownDecisions}
					itemHref={(key) => itemHref(key, new Set([i.key]), epicKeys)}
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
					panel="initiative-discussion-panel"
					bind:selectedTabId={discussionTab}
				/>
				<div role="tabpanel" tabindex="0" id="initiative-discussion-panel" class="tabpanel" aria-labelledby={discussionTab}>
					<!-- Comments stay mounted, so a half-written comment survives a switch to Activity and back. -->
					<div hidden={discussion !== 'comments'}>
						<Comments
							itemKey={i.key}
							{comments}
							onposted={(t) => {
								// A reload started before the post could answer with the thread before it; reloading now supersedes it.
								comments = t;
								reload();
							}}
						/>
					</div>
					<!-- Activity only while selected: hidden, its actor titles would still match getByTitle(owner). -->
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
				{#if decisionError}<p class="error" role="alert">{decisionError}</p>{/if}
				<button type="submit" class="primary" disabled={decisionBusy || !decision.title.trim() || !decision.decider.trim()}>Require decision</button>
			</form>
		</Dialog>
	{/if}

	{#if marking}
		<MarkUrgentDialog
			item={i.key}
			initial={i.urgent_mark?.reason ?? null}
			{busy}
			error={markError}
			onsubmit={markUrgent}
			oncancel={() => {
				if (!busy) marking = false;
			}}
		/>
	{/if}

	{#if creatingEpic !== null}
		<Dialog title="New epic" onclose={() => (creatingEpic = null)} closable={!epicFormBusy}>
			{#key key}
				<NewEpicForm
					initiative={key}
					bind:busy={epicFormBusy}
					oncreated={epicCreated(creatingEpic)}
				/>
			{/key}
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
	.epic-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 10px;
		margin: 0;
		padding: 0;
		list-style: none;
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
