<!-- web/src/lib/components/ReportDialog.svelte: "Report something" from any page. The draft lives in
     #lib/report.svelte.ts; this component only edits and sends it. Spec: Report from anywhere, 2026-11-09. -->
<script lang="ts">
	import { tick, untrack } from 'svelte';
	import { api, unwrap, ApiError } from '#lib/api/client.ts';
	import type { SimilarStory, Story } from '#lib/api/types.ts';
	import { COLUMN_LABEL, type Kind } from '#lib/board.ts';
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

	const shown = $derived(report.open && report.draft !== null);

	// Each time the dialog opens: focus the title, and re-run the duplicate check for a title already typed (a resume).
	$effect(() => {
		if (!shown) return;
		untrack(() => {
			similar = { state: 'idle', hits: [] };
			confirmResend = false;
			tick().then(() => {
				if (!d?.leftFrom) titleEl?.focus();
			});
			if (d && points(d.title.trim()) >= MIN) titleChanged(0);
		});
		return () => {
			clearTimeout(timer);
			seq++; // an answer arriving after close is dropped
		};
	});

	// Back from a similar story: focus the hit the reporter left from, once the list is there.
	$effect(() => {
		if (!shown || similar.state !== 'done') return;
		const from = d.leftFrom;
		if (!from) return;
		tick().then(() => {
			[...document.querySelectorAll<HTMLElement>('[data-hit]')].find((b) => b.dataset.hit === from)?.focus();
			if (report.draft) report.draft.leftFrom = null;
		});
	});

	function chooseKind(k: Kind) {
		if (isUntouched(d.description)) d.description = templateFor(k);
		d.kind = k;
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
		if (!d.kind) {
			error = 'Choose what kind of thing this is.';
			return;
		}
		if (!d.title.trim()) return;
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
		busy = true;
		error = null;
		try {
			const s = await unwrap(api.POST('/api/v1/stories', { body: { ...body, idempotency_key } }));
			discardDraft();
			oncreated(s, epicTitle);
		} catch (err) {
			// A transport failure means the answer was lost, not that nothing was written.
			if (err instanceof ApiError && err.code === 'transport') {
				if (report.draft) report.draft.uncertain = true;
				error = "We couldn't confirm the report was filed. Send again to retry safely.";
			} else {
				error = errorMessage(err); // the server answered and wrote nothing
			}
		} finally {
			busy = false;
			confirmResend = false;
		}
	}

	const hitState = (h: SimilarStory): string => h.resolution ?? COLUMN_LABEL[h.delivery];
</script>

{#if shown}
	<Dialog title="Report something" closable={!busy} onclose={() => (report.open = false)}>
		<form aria-label="Report" onsubmit={submit} class="report">
			<label class="field"
				>Title
				<input bind:this={titleEl} bind:value={() => report.draft?.title ?? '', (v) => report.draft && (report.draft.title = v)} oninput={() => titleChanged()} maxlength={MAX} required disabled={busy} />
			</label>
			{#if points(d.title) >= COUNT_FROM}<p class="counter">{points(d.title)} / {MAX}</p>{/if}

			<div class="similar" aria-live="polite">
				{#if similar.state === 'checking' && similar.hits.length === 0}
					<p class="hint">Checking for similar work…</p>
				{:else if similar.state === 'failed'}
					<p class="hint">Couldn't check similar work. You can still submit.</p>
				{:else if similar.hits.length > 0}
					<p class="hint">Already reported?</p>
					<ul>
						{#each similar.hits as h (h.key)}
							<li>
								<button type="button" class="ghost" data-hit={h.key} onclick={() => view(h.key)}>
									<Glyph level={levelOfKind(h.kind)} />{h.key} {h.title} <span class="state">({hitState(h)})</span>
								</button>
							</li>
						{/each}
					</ul>
				{:else if similar.state === 'done'}
					<p class="hint">Nothing similar found.</p>
				{/if}
			</div>

			<fieldset>
				<legend>What kind of thing is it?</legend>
				{#each TYPES as t (t.kind)}
					<label class="type"
						><input type="radio" name="kind" checked={d.kind === t.kind} onchange={() => chooseKind(t.kind)} disabled={busy} />
						{t.label} <span class="help">{t.help}</span></label
					>
				{/each}
				<p class="hint">Pick the closest fit; the team can change it.</p>
			</fieldset>
			<label class="field">Description <textarea bind:value={() => report.draft?.description ?? '', (v) => report.draft && (report.draft.description = v)} rows="6" disabled={busy}></textarea></label>
			<p class="from">Reported from: <a href={d.origin.path}>{d.origin.label}</a></p>
			{#if d.epic}
				<p class="chip">
					<Glyph level="epic" />Files directly under {d.epic.key} {d.epic.title}, skipping triage
					<button type="button" class="ghost" onclick={() => (d.epic = null)} disabled={busy}>Send to triage instead</button>
				</p>
			{:else if d.origin.path.startsWith('/epics/')}
				<p class="chip">Goes to triage; someone will place it.</p>
			{/if}
			{#if error}<p class="error" role="alert">{error}</p>{/if}
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
