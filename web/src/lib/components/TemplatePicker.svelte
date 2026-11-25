<!-- web/src/lib/components/TemplatePicker.svelte: the "Template" select of a create form. It loads its level's
	brief templates, preselects the one the server marked as default and puts its body in the bound brief text.
	Choosing another entry replaces the text only while it is untouched (#lib/templates.ts); otherwise it asks, and
	typed text is never discarded without that answer. A draft that is still exactly a template's body is known as
	that template. It renders nothing when the level has no template. `ready` tells the form that the list has
	arrived or failed: the form must not be sent before, because this component changes the text when it does. The
	form sends the text as the brief and nothing about templates (#lib/forms.ts). Mount it anew for each new draft. -->
<script lang="ts">
	import { api, unwrap } from '#lib/api/client.ts';
	import type { BriefTemplate, BriefTemplateLevel } from '#lib/api/types.ts';
	import { defaultOf, pick, startedFrom } from '#lib/templates.ts';

	let {
		level,
		text = $bindable(),
		ready = $bindable(false),
		prefill = true,
		disabled = false
	}: {
		level: BriefTemplateLevel;
		text: string;
		/** False while the list is being loaded; true once it arrived or the load failed. Bind it, and do not send
		 *  the form while it is false. */
		ready?: boolean;
		/** Put the default's body into a blank brief when the list arrives. A form passes false for a draft whose
		 *  templates have already loaded once, so a default the author removed does not come back and a retry of a
		 *  sent draft is the same create. */
		prefill?: boolean;
		disabled?: boolean;
	} = $props();

	let templates = $state<BriefTemplate[]>([]);
	let failed = $state(false);
	/** The entry the text came from: a template's id as text, or '' for "No template". */
	let chosen = $state('');
	/** The body the picker last put in the field (or found there, for a kept draft). */
	let applied = $state('');
	/** The entry the author asked for while the field held text of their own. */
	let asking = $state<{ id: string; name: string | null; body: string } | null>(null);

	const find = (id: string) => templates.find((t) => String(t.id) === id);
	/** How long the picker waits for its list before it gives up and lets the form be sent without templates. */
	const GIVE_UP_MS = 10_000;

	function fill(id: string, body: string) {
		chosen = id;
		text = body;
		applied = body;
		asking = null;
	}

	// Loads once per mount (and again if the level changes). The answer is handled after the effect has run, so
	// reading `text` and `prefill` there does not make the effect depend on them. This is the only place the
	// picker changes the text without the author's hand, and `ready` is false until it is over. A list that has
	// not arrived after GIVE_UP_MS is treated as a failed load, and an answer after that is ignored.
	$effect(() => {
		let live = true;
		ready = false;
		const giveUp = setTimeout(() => {
			if (!live) return;
			live = false;
			failed = true;
			ready = true;
		}, GIVE_UP_MS);
		unwrap(api.GET('/api/v1/brief-templates', { params: { query: { level } } })).then(
			(list) => {
				if (!live) return;
				clearTimeout(giveUp);
				templates = list;
				const from = startedFrom(list, text);
				const d = defaultOf(list);
				if (from) {
					// A draft kept from a cancelled dialog whose text is still a template's own: it is that
					// template, untouched. Nothing is written to the text.
					chosen = String(from.id);
					applied = from.body;
				} else if (prefill && d && pick({ text, applied }, d.body).kind === 'fill') {
					// Only an untouched field takes the default: text of the author's stays.
					fill(String(d.id), d.body);
				}
				ready = true;
			},
			() => {
				if (!live) return;
				clearTimeout(giveUp);
				// The form works without templates: the author writes the brief.
				failed = true;
				ready = true;
			}
		);
		return () => {
			live = false;
			clearTimeout(giveUp);
		};
	});

	function onchange(e: Event) {
		const el = e.currentTarget as HTMLSelectElement;
		const id = el.value;
		const t = find(id);
		const body = t?.body ?? '';
		if (pick({ text, applied }, body).kind === 'fill') {
			fill(id, body);
			return;
		}
		// The select keeps showing the entry the text came from until the author answers.
		el.value = chosen;
		asking = { id, name: t?.name ?? null, body };
	}
</script>

{#if templates.length}
	<label class="field">
		Template
		<select value={chosen} {onchange} {disabled}>
			<option value="">No template</option>
			{#each templates as t (t.id)}
				<option value={String(t.id)}>{t.name}{t.is_default ? ' (default)' : ''}</option>
			{/each}
		</select>
	</label>
	{#if asking}
		<p class="ask" role="alert">
			{#if asking.name === null}
				Remove what you have written?
				<button type="button" {disabled} onclick={() => asking && fill(asking.id, asking.body)}>Remove my text</button>
			{:else}
				Replace what you have written with the template "{asking.name}"?
				<button type="button" {disabled} onclick={() => asking && fill(asking.id, asking.body)}>Replace</button>
			{/if}
			<button type="button" {disabled} onclick={() => (asking = null)}>Keep my text</button>
		</p>
	{/if}
{:else if failed}
	<p class="muted">Templates could not be loaded. You can still write the brief yourself.</p>
{/if}

<style>
	.ask {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		align-items: center;
		margin: 0;
	}
	.muted {
		margin: 0;
		color: var(--muted);
	}
</style>
