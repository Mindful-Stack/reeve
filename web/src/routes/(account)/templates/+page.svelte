<!-- web/src/routes/(account)/templates/+page.svelte: brief templates, by level. Every member reads them, each body
	rendered as a brief is, inside its row. An admin also creates, edits (only the fields changed, under the version
	read), removes and marks the default; every control follows isAdmin() and the server decides. A template only
	prefills a new brief: nothing here touches an existing one. -->
<script lang="ts">
	import { api, ApiError, Conflict, unwrap } from '#lib/api/client.ts';
	import type { BriefTemplate, BriefTemplateLevel } from '#lib/api/types.ts';
	import Brief from '#lib/components/Brief.svelte';
	import { onLive } from '#lib/events.svelte.ts';
	import { mergeDraft, templateEdit, templateFields, TEMPLATE_SAME, type TemplateFields } from '#lib/forms.ts';
	import { errorMessage } from '#lib/format.ts';
	import { isAdmin } from '#lib/me.svelte.ts';
	import Notice from '#lib/ui/Notice.svelte';
	import PageHeader from '#lib/ui/PageHeader.svelte';

	const LEVELS: { level: BriefTemplateLevel; heading: string; none: string }[] = [
		{ level: 'initiative', heading: 'Initiative templates', none: 'No initiative templates yet.' },
		{ level: 'epic', heading: 'Epic templates', none: 'No epic templates yet.' }
	];
	/** The planner raises the advisory finding `brief_cites_without_decision` on a brief that says one of these
	 *  words (reeve-planning, domain/findings.rs), so a template that says one raises it on every item made from it
	 *  that has no required decision on it or above it. */
	const CITES_HINT = "Avoid the words 'decided', 'Jira' and 'Confluence' in a template: Reeve would note on each initiative or epic made from it that its brief cites them without a required decision.";
	const CHANGED = 'Someone else changed this template while you were editing.';

	let templates = $state<BriefTemplate[] | null>(null);
	let loadError = $state<string | null>(null);
	let notice = $state<string | null>(null);
	let busy = $state(false);
	/** The "New template" form is open. */
	let adding = $state(false);
	let draft = $state<{ level: BriefTemplateLevel; name: string; body: string }>({ level: 'initiative', name: '', body: '' });
	/** The template being edited: the version its save names, and its fields as the editor last took them from the
	 *  server (when it opened, or when a conflict was merged). `edit` is what the admin has typed. */
	let editing = $state<{ id: number; version: number; from: TemplateFields } | null>(null);
	let edit = $state<TemplateFields>({ name: '', body: '' });
	/** The template whose removal awaits its confirmation. */
	let removing = $state<number | null>(null);

	function failure(e: unknown): string {
		if (e instanceof Conflict) return CHANGED;
		if (e instanceof ApiError && e.code === 'forbidden') return 'Only an admin can change templates.';
		return errorMessage(e);
	}

	function closeEdit() {
		editing = null;
		edit = { name: '', body: '' };
	}

	let latest = 0;
	/** The first list has not been shown yet. */
	let first = true;
	/** Loads the list. A write's reload and a live reload can overlap: only the answer of the request made last is
	 *  shown, an older one is dropped. Answers true when this request's list is the one now shown; false when it
	 *  failed or was overtaken. */
	async function reload(): Promise<boolean> {
		const seq = ++latest;
		let list: BriefTemplate[];
		try {
			list = await unwrap(api.GET('/api/v1/brief-templates'));
		} catch (e) {
			if (seq === latest) loadError = failure(e);
			return false;
		}
		if (seq !== latest) return false;
		templates = list;
		loadError = null;
		// An instance without a single template: the admin's first step is to add one, so the form starts open.
		if (first) {
			first = false;
			if (list.length === 0 && isAdmin()) adding = true;
		}
		// A template someone removed takes its open editor or confirmation with it.
		const open = editing;
		if (open && !list.some((t) => t.id === open.id)) {
			closeEdit();
			notice = 'The template you were editing was removed.';
		}
		if (removing !== null && !list.some((t) => t.id === removing)) removing = null;
		return true;
	}
	reload();
	// Another admin's change reaches an open page: every template write is an activity entry. A live reload never
	// moves an open editor's base version: only a conflict does (see `save`).
	onLive(reload);

	// A viewer who stops being an admin loses what is open, at once; the markup checks isAdmin() too, so nothing of
	// it is drawn in between, and nothing comes back if the role returns.
	$effect(() => {
		if (!isAdmin()) {
			closeEdit();
			removing = null;
			adding = false;
		}
	});

	/** How a write ended. `conflict` carries whether the reload after it is the list now shown. */
	type Outcome = { kind: 'done' } | { kind: 'conflict'; fresh: boolean } | { kind: 'failed' } | { kind: 'busy' };

	/** One write, then the list again whether it worked or not (a refusal may mean the list is stale). A failure is
	 *  shown. The caller is told how it ended, because only a conflict may move an editor's base. */
	async function write(what: () => Promise<unknown>): Promise<Outcome> {
		if (busy || !isAdmin()) return { kind: 'busy' };
		busy = true;
		notice = null;
		let out: Outcome = { kind: 'done' };
		try {
			await what();
		} catch (e) {
			out = e instanceof Conflict ? { kind: 'conflict', fresh: false } : { kind: 'failed' };
			notice = failure(e);
		}
		const fresh = await reload();
		if (out.kind === 'conflict') out = { kind: 'conflict', fresh };
		busy = false;
		return out;
	}

	async function create(e: SubmitEvent) {
		e.preventDefault();
		const body = { level: draft.level, name: draft.name, body: draft.body };
		const out = await write(() => unwrap(api.POST('/api/v1/brief-templates', { body })));
		if (out.kind === 'done') {
			draft = { level: draft.level, name: '', body: '' };
			adding = false;
		}
	}

	function startEdit(t: BriefTemplate) {
		if (editing) return; // one editor at a time: its text is not discarded for another row's
		const from = templateFields(t);
		editing = { id: t.id, version: t.version, from };
		edit = { ...from };
		removing = null;
	}

	async function save(e: SubmitEvent) {
		e.preventDefault();
		if (!editing) return;
		const { id, version, from } = editing;
		// Only the fields changed since the editor took them from the server are sent.
		const change = templateEdit(from, edit);
		if (change.kind === 'unchanged') {
			closeEdit();
			return;
		}
		if (change.kind === 'invalid') {
			notice = change.error;
			return;
		}
		const out = await write(() => unwrap(api.PATCH('/api/v1/brief-templates/{id}', { params: { path: { id } }, body: { version, ...change.body } })));
		if (out.kind === 'done') {
			closeEdit();
			return;
		}
		// Any failure that is not a conflict (the network, a refused name, a lost role) leaves the editor exactly as
		// it was: the same base version and the same opening fields, although the list was reloaded. A retry is
		// then judged by the server against the version this editor read.
		if (out.kind !== 'conflict') return;
		if (!out.fresh) {
			notice = `${CHANGED} The list could not be reloaded; save again to try once more.`;
			return;
		}
		// A conflict, and the list now shown was read after it: the fields the admin changed stay, every other field
		// takes what was reloaded, and the base becomes the reloaded version. Saving again is then a decision to
		// replace the other change to the fields this admin changed, and the notice says so.
		const now = templates?.find((t) => t.id === id);
		if (!now || !editing) return; // removed meanwhile: `reload` closed the editor and said so
		const server = templateFields(now);
		edit = mergeDraft(TEMPLATE_SAME, from, edit, server);
		editing = { id, version: now.version, from: server };
		notice = `${CHANGED} What you changed is still here; everything else now shows their version. Where you both changed the same thing, saving again replaces their change.`;
	}

	const mark = (t: BriefTemplate, on: boolean) =>
		write(() =>
			unwrap(
				on
					? api.POST('/api/v1/brief-templates/{id}/default', { params: { path: { id: t.id } } })
					: api.DELETE('/api/v1/brief-templates/{id}/default', { params: { path: { id: t.id } } })
			)
		);

	async function remove(id: number) {
		const out = await write(() => unwrap(api.DELETE('/api/v1/brief-templates/{id}', { params: { path: { id } } })));
		if (out.kind !== 'busy') removing = null;
	}
</script>

<svelte:head><title>Templates · Reeve</title></svelte:head>

<PageHeader title="Brief templates" />
<p class="muted">
	A template is the text a new initiative's or epic's brief starts from. The default template of a level is filled in
	when someone starts a new initiative or epic in Reeve's form; they can pick another one or none. The <code>reeve</code>
	command line starts from the default too when no brief is given. An initiative or epic created from a chat app, a
	plan or an import starts empty. A template only fills the new brief: changing or removing a template changes no brief
	that was made from it.
</p>

{#if notice}<Notice tone="error" ondismiss={() => (notice = null)}>{notice}</Notice>{/if}
{#if loadError}<Notice tone="error" onretry={reload}>Could not load the templates: {loadError}</Notice>{/if}

{#if templates}
	<div aria-busy={busy}>
		{#if isAdmin()}
			<details class="new" bind:open={adding}>
				<summary>New template</summary>
				<form class="create" aria-label="New template" onsubmit={create}>
					<label class="field">
						Level
						<select bind:value={draft.level} disabled={busy}>
							<option value="initiative">Initiative</option>
							<option value="epic">Epic</option>
						</select>
					</label>
					<label class="field">Name <input bind:value={draft.name} required disabled={busy} /></label>
					<label class="field">Template text (Markdown) <textarea bind:value={draft.body} rows="10" required disabled={busy}></textarea></label>
					<p class="muted hint">{CITES_HINT}</p>
					{#if draft.body.trim()}
						<div class="preview" role="group" aria-label="Preview">
							<p class="muted">Preview</p>
							<Brief markdown={draft.body} />
						</div>
					{/if}
					<button type="submit" class="primary" disabled={busy || !draft.name.trim() || !draft.body.trim()}>Create template</button>
				</form>
			</details>
		{/if}

		{#each LEVELS as { level, heading, none } (level)}
			{@const mine = templates.filter((t) => t.level === level)}
			<section aria-label={heading}>
				<h2>{heading}</h2>
				{#if mine.length === 0}<p class="muted">{none}{isAdmin() ? ' Add one with "New template" above.' : ' An admin can add one.'}</p>{/if}
				{#each mine as t (t.id)}
					<article data-template={t.id}>
						{#if isAdmin() && editing?.id === t.id}
							<form class="edit" aria-label="Edit template" onsubmit={save}>
								<label class="field">Name <input bind:value={edit.name} required disabled={busy} /></label>
								<label class="field">Template text (Markdown) <textarea bind:value={edit.body} rows="12" required disabled={busy}></textarea></label>
								<p class="muted hint">{CITES_HINT}</p>
								{#if edit.body.trim()}
									<div class="preview" role="group" aria-label="Preview">
										<p class="muted">Preview</p>
										<Brief markdown={edit.body} />
									</div>
								{/if}
								<div class="actions">
									<button type="button" disabled={busy} onclick={closeEdit}>Cancel</button>
									<button type="submit" class="primary" disabled={busy}>Save</button>
								</div>
							</form>
						{:else}
							<details>
								<summary>
									<span class="name">{t.name}</span>
									{#if t.is_default}<span class="mark">Default</span>{/if}
								</summary>
								<div class="body"><Brief markdown={t.body} /></div>
							</details>
							{#if isAdmin()}
								<div class="actions">
									{#if t.is_default}
										<button type="button" disabled={busy} onclick={() => mark(t, false)}>Stop using as default</button>
									{:else}
										<button type="button" disabled={busy} onclick={() => mark(t, true)}>Use as default</button>
									{/if}
									<!-- One editor at a time: a second "Edit" would discard what the first holds. -->
									<button
										type="button"
										disabled={busy || editing !== null}
										title={editing !== null ? 'Save or cancel the template you are editing first.' : undefined}
										onclick={() => startEdit(t)}>Edit</button
									>
									<button type="button" disabled={busy} onclick={() => (removing = t.id)}>Remove</button>
								</div>
								{#if removing === t.id}
									<p class="confirm" role="alert">
										Remove the template "{t.name}"? Briefs already made from it stay as they are.
										<button type="button" disabled={busy} onclick={() => (removing = null)}>Cancel</button>
										<button type="button" class="danger" disabled={busy} onclick={() => remove(t.id)}>Remove template</button>
									</p>
								{/if}
							{/if}
						{/if}
					</article>
				{/each}
			</section>
		{/each}
	</div>
{:else if !loadError}
	<p class="muted">Loading…</p>
{/if}

<style>
	section,
	.new {
		margin: 0 0 24px;
	}
	.new > summary,
	article summary {
		cursor: pointer;
	}
	/* One column: the summary, then the admin's buttons on a row of their own, then the body at the row's full
	   width. A column of buttons beside the summary took its width first: a name broke letter by letter on a phone
	   and an opened table lost that width on a desktop. */
	article {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 8px;
		margin: 0 0 8px;
		padding: 8px 12px;
		border: 1px solid var(--line);
		border-radius: 6px;
		min-width: 0;
	}
	article details {
		min-width: 0;
	}
	.name {
		font-weight: 600;
		overflow-wrap: anywhere;
	}
	.mark {
		margin-left: 8px;
		padding: 0 6px;
		border: 1px solid var(--line);
		border-radius: 3px;
		font-size: 13px;
	}
	.actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
	.body,
	.preview {
		margin-top: 8px;
		overflow-x: auto;
	}
	.preview {
		padding: 8px 12px;
		border: 1px dashed var(--line);
		border-radius: 6px;
	}
	.preview > .muted,
	.hint {
		margin: 0 0 8px;
		font-size: 13px;
	}
	.create,
	.edit {
		display: grid;
		gap: 12px;
		margin-top: 12px;
	}
	.create button {
		justify-self: start;
	}
	textarea {
		font: inherit;
	}
	.confirm {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		margin: 0;
	}
	.muted {
		color: var(--muted);
	}
	.danger {
		border-color: var(--danger);
		color: var(--danger);
	}
</style>
