<script lang="ts">
	import { tick } from 'svelte';
	import Picker from '#lib/ui/Picker.svelte';
	import { keyed } from '#lib/picker.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { api, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import type { Epic, Initiative } from '#lib/api/types.ts';
	import { epicCreate, type EpicDraft } from '#lib/forms.ts';
	import { CreateCycle } from '#lib/idempotency.ts';
	import { session } from '#lib/me.svelte.ts';
	import { planningOrder } from '#lib/planning.ts';
	import ImageUploadsRow from '#lib/components/ImageUploadsRow.svelte';
	import { ImageUploads, attachable } from '#lib/attach.svelte.ts';

	// Creates an epic. On the initiative page the parent is fixed (`initiative`); on the backlog a picker of
	// `initiatives` plus "none" picks it. Mount it under `{#key}` of the page's item so a draft never moves to another item.
	let {
		initiative,
		initiatives = [],
		busy = $bindable(false),
		oncreated
	}: {
		initiative?: string;
		initiatives?: Initiative[];
		/** A create is in flight; bind it to keep the form's dialog from closing mid-create (`closable={!busy}`). */
		busy?: boolean;
		oncreated: (e: Epic) => Promise<void> | void;
	} = $props();

	/** The picker lists the initiatives in the planning page's order. */
	const ordered = $derived(planningOrder(session.config?.horizon_buckets ?? [], initiatives));

	let draft = $state<EpicDraft>({ title: '', brief: '', initiative: '' });
	/** This form instance's sends: a new key after each success, the same key for a retry, one request per
	 *  double submit. */
	const cycle = new CreateCycle();
	let error = $state<string | null>(null);
	let briefEl = $state<HTMLTextAreaElement | null>(null);
	const uploads = new ImageUploads(
		() => draft.brief,
		(t) => (draft.brief = t)
	);
	let formEl: HTMLFormElement | undefined;
	let titleEl: HTMLInputElement | undefined;

	/** False once this form is unmounted: a send that answers after that leaves `busy` (bound to the page, which may show
	 *  another form by now), the error and focus alone. The captured `oncreated` still runs, so the page reloads. */
	let mounted = true;
	$effect(() => () => {
		mounted = false;
	});

	/** The fields were disabled while sending, so focus fell to the page: put it back on Title, unless the user moved it. */
	function refocus() {
		void tick().then(() => {
			if (focusIsFree(formEl)) titleEl?.focus();
		});
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (cycle.inFlight || uploads.busy) return;
		// The callback of the parent this submit belongs to: once this form is unmounted (its dialog closed while the create
		// was in flight), reading the prop again may give the callback of whatever the parent shows now.
		const done = oncreated;
		const p = epicCreate({ ...draft, initiative: initiative ?? draft.initiative });
		if (!p.ok) {
			error = p.error;
			return;
		}
		busy = true;
		error = null;
		const out = await cycle.send(p.text, (idempotency_key) =>
			unwrap(api.POST('/api/v1/epics', { body: { ...p.body, ...uploads.body(), idempotency_key } }))
		);
		if (out.kind === 'sent' && !mounted) await done(out.value);
		if (!mounted) return;
		busy = false;
		if (out.kind === 'sent') {
			draft = { title: '', brief: '', initiative: draft.initiative };
			uploads.reset();
			refocus();
			await done(out.value);
		} else if (out.kind === 'failed') {
			error = errorMessage(out.error); // the draft and its key stay
			refocus();
		}
	}
</script>

<form bind:this={formEl} class="create-form" aria-label="New epic" onsubmit={submit}>
	<label class="field">Title <input bind:this={titleEl} bind:value={draft.title} required disabled={busy} /></label>
	{#if initiative === undefined}
		<label class="field">
			Initiative
			<Picker label="Initiative" options={keyed(ordered)} bind:value={draft.initiative} empty="No initiative" disabled={busy} />
		</label>
	{/if}
	<div class="field">
		<label class="field">Brief (Markdown) <textarea bind:this={briefEl} bind:value={draft.brief} rows="3" disabled={busy} {@attach attachable(uploads)}></textarea></label>
		<ImageUploadsRow {uploads} field={briefEl} disabled={busy} />
	</div>
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<button type="submit" class="primary" disabled={busy || uploads.busy || !draft.title.trim()}>{uploads.busy ? 'Uploading…' : 'Create epic'}</button>
</form>

<style>
	.create-form {
		display: grid;
		gap: 12px;
	}
	.create-form button {
		justify-self: start;
	}
	textarea {
		font: inherit;
	}
	.error {
		margin: 0;
		color: var(--danger);
	}
</style>
