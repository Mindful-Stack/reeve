<!-- web/src/lib/components/ReportDialog.svelte: "Report something" from any page. The draft lives in
     #lib/report.svelte.ts; this component only edits and sends it. Spec: Report from anywhere, 2026-11-09. -->
<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { api, unwrap, ApiError, Refused } from '#lib/api/client.ts';
	import type { SimilarStory, Story } from '#lib/api/types.ts';
	import { COLUMN_LABEL, fullLabel, type Kind } from '#lib/board.ts';
	import { focusIsFree } from '#lib/focus.ts';
	import { errorMessage } from '#lib/format.ts';
	import { levelOfKind } from '#lib/glyphs.ts';
	import { draftKey } from '#lib/idempotency.ts';
	import { report, discardDraft, isUntouched, payloadOf, payloadText, templateFor } from '#lib/report.svelte.ts';
	import Dialog from '#lib/ui/Dialog.svelte';
	import Glyph from './Glyph.svelte';

	let {
		oncreated,
		onviewstory
	}: { oncreated: (s: Story, epicTitle: string | null) => void; onviewstory: (key: string) => void } = $props();

	const TYPES: { kind: Kind; label: string; help: string }[] = [
		{ kind: 'bug', label: "Something's broken", help: 'It worked before, or should, and doesn’t.' },
		{ kind: 'story', label: 'Idea or change', help: 'Something new, or something that should work differently.' },
		{ kind: 'chore', label: 'Maintenance task', help: 'Cleanup, upgrades or routine upkeep.' }
	];
	const id = $props.id();
	const MIN = 8;
	const COUNT_FROM = 180;
	const MAX = 200;
	const points = (s: string): number => [...s].length;

	// The form renders only while a draft exists (see the {#if} below): Discard and a create set it to null.
	const d = $derived(report.draft!);
	/** An edit after an uncertain send waits for "Send anyway". */
	let confirmResend = $state(false);
	let busy = $state(false);
	let error = $state<string | null>(null);
	let similar = $state<{ state: 'idle' | 'checking' | 'done' | 'failed'; hits: SimilarStory[] }>({ state: 'idle', hits: [] });
	let seq = 0;
	let timer: ReturnType<typeof setTimeout> | undefined;
	let titleEl: HTMLInputElement | undefined = $state();
	let formEl: HTMLFormElement | undefined = $state();
	/** The current error is the missing kind (linked to the fieldset). */
	let kindError = $state(false);
	const LOST = "We couldn't confirm the report was filed. Send again to retry safely.";

	const shown = $derived(report.open && report.draft !== null);

	// Each time the dialog opens: focus the title, and re-run the duplicate check for a title already typed (a resume).
	$effect(() => {
		if (!shown) return;
		untrack(() => {
			similar = { state: 'idle', hits: [] };
			confirmResend = false;
			kindError = false;
			// A kept draft whose send was lost still says so; any other error belongs to a draft that is gone.
			error = d?.uncertain ? LOST : null;
			// The title is focused by Dialog's focusFirst (it is the first field); only a resume moves focus after that.
			if (d && points(d.title.trim()) >= MIN) titleChanged(0);
		});
		return () => {
			clearTimeout(timer);
			seq++; // an answer arriving after close is dropped
		};
	});

	// Back from a similar story: focus the hit the reporter left from once the answer is there; if it is not in the answer
	// (or the check failed), focus the title.
	$effect(() => {
		if (!shown || (similar.state !== 'done' && similar.state !== 'failed')) return;
		const from = d.leftFrom;
		if (!from) return;
		const hits = similar.hits;
		tick().then(() => {
			const el = hits.some((h) => h.key === from)
				? [...document.querySelectorAll<HTMLElement>('[data-hit]')].find((b) => b.dataset.hit === from)
				: undefined;
			(el ?? titleEl)?.focus();
			if (report.draft) report.draft.leftFrom = null;
		});
	});

	/** The fields were disabled while sending, so focus fell to the page: put it back on the title, unless the user moved it. */
	function refocus() {
		void tick().then(() => {
			if (focusIsFree(formEl)) titleEl?.focus();
		});
	}

	/** Trims the title to 200 code points from the text just typed (before the caret), keeping what follows it. */
	function fit(el: HTMLInputElement) {
		const cps = [...el.value];
		if (cps.length <= MAX) return;
		const caret = [...el.value.slice(0, el.selectionStart ?? el.value.length)].length;
		const keep = Math.max(0, caret - (cps.length - MAX));
		el.value = [...cps.slice(0, keep), ...cps.slice(caret)].join('');
		const pos = cps.slice(0, keep).join('').length;
		el.setSelectionRange(pos, pos);
	}

	function titleInput(e: Event) {
		const el = e.currentTarget as HTMLInputElement;
		if (!(e as InputEvent).isComposing) fit(el);
		d.title = el.value;
		titleChanged();
	}

	function titleComposed(e: Event) {
		const el = e.currentTarget as HTMLInputElement;
		fit(el);
		d.title = el.value;
		titleChanged();
	}

	function chooseKind(k: Kind) {
		if (isUntouched(d.description)) d.description = templateFor(k);
		d.kind = k;
		if (kindError) {
			kindError = false;
			error = null;
		}
	}

	function titleChanged(delay = 300) {
		clearTimeout(timer);
		const mine = ++seq; // any older answer, in flight or not, is now stale
		const q = d.title.trim();
		if (points(q) < MIN) {
			similar = { state: 'idle', hits: [] };
			return;
		}
		timer = setTimeout(async () => {
			similar = { state: 'checking', hits: similar.hits };
			try {
				const hits = await unwrap(api.GET('/api/v1/stories/similar', { params: { query: { q } } }));
				if (mine === seq) similar = { state: 'done', hits };
			} catch {
				if (mine === seq) similar = { state: 'failed', hits: [] };
			}
		}, delay);
	}

	function view(key: string) {
		d.leftFrom = key;
		report.open = false;
		onviewstory(key);
	}

	async function submit(e: SubmitEvent) {
		e.preventDefault();
		if (busy) return;
		kindError = false;
		if (!d.title.trim()) {
			error = 'Give it a title.';
			titleEl?.focus();
			return;
		}
		if (!d.kind) {
			error = 'Choose what kind of thing this is.';
			kindError = true;
			return;
		}
		const body = payloadOf(d);
		const text = payloadText(d);
		// After a lost answer, a changed payload would get a new key and could file a second item: ask first.
		if (d.uncertain && d.key.sent !== text && !confirmResend) {
			error = 'Your last report may have been filed. Check before sending again?';
			titleChanged(); // re-run the similar list so the reporter can look
			confirmResend = true;
			return;
		}
		const idempotency_key = draftKey(d.key, text);
		const epicTitle = d.epic?.title ?? null;
		const sent = report.draft; // the draft this send belongs to; the page may start another while it is in flight
		busy = true;
		error = null;
		try {
			const s = await unwrap(api.POST('/api/v1/stories', { body: { ...body, idempotency_key } }));
			if (report.draft === sent) discardDraft();
			oncreated(s, epicTitle);
		} catch (err) {
			const mine = report.draft === sent;
			// A transport failure means the answer was lost, not that nothing was written.
			if (err instanceof ApiError && err.code === 'transport') {
				if (mine && sent) sent.uncertain = true;
				if (mine) error = LOST;
			} else if (mine) {
				// The server answered and wrote nothing.
				error = err instanceof Refused ? err.reasons.map((r) => fullLabel(r)).join('; ') : errorMessage(err);
			}
			refocus();
		} finally {
			busy = false;
			confirmResend = false;
		}
	}

	const hitState = (h: SimilarStory): string =>
		h.delivery === 'done' && h.resolution ? `done, ${h.resolution}` : COLUMN_LABEL[h.delivery];
	const status = $derived(
		similar.state === 'checking'
			? 'Checking similar work…'
			: similar.state === 'failed'
				? "Couldn't check similar work. You can still submit."
				: similar.state === 'done'
					? similar.hits.length === 0
						? 'Nothing similar found'
						: `${similar.hits.length} similar ${similar.hits.length === 1 ? 'item' : 'items'}`
					: ''
	);
</script>

{#if shown}
	<Dialog title="Report something" closable={!busy} onclose={() => (report.open = false)}>
		<form bind:this={formEl} aria-label="Report" onsubmit={submit} class="report">
			<label class="field"
				>Title
				<input
					bind:this={titleEl}
					value={d.title}
					oninput={titleInput}
					oncompositionend={titleComposed}
					aria-describedby={points(d.title) >= COUNT_FROM ? `${id}-count` : undefined}
					required
					disabled={busy}
				/>
			</label>
			{#if points(d.title) >= COUNT_FROM}<p class="counter" id="{id}-count">{points(d.title)} / {MAX}</p>{/if}

			<p class="hint status" aria-live="polite">{status}</p>
			{#if similar.hits.length > 0}
				<div class="similar">
					<p class="hint">Already reported?</p>
					<ul>
						{#each similar.hits as h (h.key)}
							<li>
								<button type="button" class="ghost" data-hit={h.key} disabled={busy} onclick={() => view(h.key)}>
									<Glyph level={levelOfKind(h.kind)} />{h.key} {h.title} <span class="state">({hitState(h)})</span>
								</button>
							</li>
						{/each}
					</ul>
				</div>
			{/if}

			<fieldset role="radiogroup" aria-required="true" aria-invalid={kindError ? 'true' : undefined} aria-describedby={kindError ? `${id}-error` : undefined}>
				<legend>What kind of thing is it?</legend>
				{#each TYPES as t (t.kind)}
					<label class="type"
						><input
							type="radio"
							name="kind"
							checked={d.kind === t.kind}
							onchange={() => chooseKind(t.kind)}
							aria-describedby={kindError ? `${id}-help-${t.kind} ${id}-error` : `${id}-help-${t.kind}`}
							disabled={busy}
						/>
						{t.label} <span class="help" id="{id}-help-{t.kind}">{t.help}</span></label
					>
				{/each}
				<p class="hint">Pick the closest fit; the team can change it.</p>
			</fieldset>
			<label class="field">Description <textarea value={d.description} oninput={(e) => (d.description = e.currentTarget.value)} rows="6" disabled={busy}></textarea></label>
			<p class="from">Reported from: <a href={d.origin.path}>{d.origin.label}</a></p>
			{#if d.epic}
				<p class="chip">
					<Glyph level="epic" />Files directly under {d.epic.key} {d.epic.title}, skipping triage
					<button type="button" class="ghost" onclick={() => (d.epic = null)} disabled={busy}>Send to triage instead</button>
				</p>
			{:else if d.origin.path.startsWith('/epics/')}
				<p class="chip">Goes to triage; someone will place it.</p>
			{/if}
			{#if error}<p class="error" id="{id}-error" role="alert">{error}</p>{/if}
			<div class="actions">
				<button type="submit" class="primary" disabled={busy}>{confirmResend ? 'Send anyway' : 'Report'}</button>
				<button type="button" class="ghost" onclick={() => discardDraft()} disabled={busy}>Discard draft</button>
			</div>
		</form>
	</Dialog>
{/if}

<style>
	.report {
		display: grid;
		gap: 12px;
	}
	.field {
		display: grid;
		gap: 4px;
	}
	.type {
		display: block;
	}
	.help,
	.hint,
	.counter,
	.state {
		color: var(--muted, inherit);
	}
	.hint,
	.counter,
	.from,
	.chip,
	.error {
		margin: 0;
	}
	.similar ul {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.error {
		color: var(--danger);
	}
	.actions {
		display: flex;
		gap: 8px;
	}
</style>
