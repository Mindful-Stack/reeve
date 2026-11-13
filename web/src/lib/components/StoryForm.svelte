<!-- web/src/lib/components/StoryForm.svelte: creates a story. On an epic page the epic is fixed (`epic`); on the backlog an
     epic picker (with "No epic") picks it. Same send cycle as NewEpicForm (one request per double submit). Mount it under
     `{#key}` of the page's item so a draft never moves to another item. -->
<script lang="ts">
	import { tick } from 'svelte';
	import Picker from '#lib/ui/Picker.svelte';
	import { keyed } from '#lib/picker.ts';
	import { api, unwrap } from '#lib/api/client.ts';
	import { errorMessage } from '#lib/format.ts';
	import type { Epic, Story } from '#lib/api/types.ts';
	import { KINDS, type Kind } from '#lib/board.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { CreateCycle } from '#lib/idempotency.ts';

	let {
		epic,
		epics = [],
		disabled = false,
		busy = $bindable(false),
		oncreated
	}: {
		epic?: string;
		epics?: Epic[];
		/** The page has another write in flight. */
		disabled?: boolean;
		/** A create is in flight; bind it to keep the form's dialog from closing mid-create (`closable={!busy}`). */
		busy?: boolean;
		oncreated: (s: Story) => Promise<void> | void;
	} = $props();

	let draft = $state<{ title: string; kind: Kind; epic: string }>({ title: '', kind: 'story', epic: '' });
	/** This form instance's sends: a new key after each success, the same key for a retry. */
	const cycle = new CreateCycle();
	let error = $state<string | null>(null);
	let formEl: HTMLFormElement | undefined;
	let titleEl: HTMLInputElement | undefined;
	const off = $derived(busy || disabled);

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
		const title = draft.title.trim();
		if (!title || cycle.inFlight || disabled) return;
		// The callback of the parent this submit belongs to: once this form is unmounted (its dialog closed while the create
		// was in flight), reading the prop again may give the callback of whatever the parent shows now.
		const done = oncreated;
		const parent = epic ?? (draft.epic || null);
		const body = { title, kind: draft.kind, epic: parent, criteria: [], labels: [] };
		busy = true;
		error = null;
		const out = await cycle.send(JSON.stringify(body), (idempotency_key) =>
			unwrap(api.POST('/api/v1/stories', { body: { ...body, idempotency_key } }))
		);
		if (out.kind === 'sent' && !mounted) await done(out.value);
		if (!mounted) return;
		busy = false;
		if (out.kind === 'sent') {
			draft = { title: '', kind: draft.kind, epic: draft.epic };
			refocus();
			await done(out.value);
		} else if (out.kind === 'failed') {
			error = errorMessage(out.error); // the draft and its key stay
			refocus();
		}
	}
</script>

<form bind:this={formEl} class="create-form" aria-label="New story" onsubmit={submit}>
	<label class="field">Title <input bind:this={titleEl} bind:value={draft.title} required disabled={off} /></label>
	<label class="field">
		Kind
		<select bind:value={draft.kind} disabled={off}>
			{#each KINDS as k (k)}<option value={k}>{k}</option>{/each}
		</select>
	</label>
	{#if epic === undefined}
		<label class="field">
			Epic
			<Picker label="Epic" options={keyed(epics)} bind:value={draft.epic} empty="No epic" disabled={off} />
		</label>
	{/if}
	{#if error}<p class="error" role="alert">{error}</p>{/if}
	<button type="submit" class="primary" disabled={off || !draft.title.trim()}>Create story</button>
</form>

<style>
	.create-form {
		display: grid;
		gap: 12px;
	}
	.create-form button {
		justify-self: start;
	}
	.error {
		margin: 0;
		color: var(--danger);
	}
</style>
