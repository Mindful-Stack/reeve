<script lang="ts">
	import { api, ApiError, Refused, unwrap } from '#lib/api/client.ts';
	import type { Comment } from '#lib/api/types.ts';
	import { fullLabel } from '#lib/board.ts';
	import { age, errorMessage, short } from '#lib/format.ts';
	import { draftKey, newDraftKey } from '#lib/idempotency.ts';
	import CommentBody from '#lib/components/CommentBody.svelte';

	// A story's comment thread, oldest first, and the form that adds to it. Posting replaces the thread with the one the
	// server returns, then tells the drawer (which reloads the activity).
	let {
		itemKey,
		comments,
		onposted
	}: {
		itemKey: string;
		comments: Comment[];
		/** Called with the thread the post returned. */
		onposted: (thread: Comment[]) => void;
	} = $props();

	let body = $state('');
	/** One idempotency key per comment being written; `draftKey` keeps it across retries of the same text only. */
	let draft = newDraftKey();
	let busy = $state(false);
	let error = $state<string | null>(null);

	async function post(e: SubmitEvent) {
		e.preventDefault();
		const text = body.trim();
		if (!text || busy) return;
		busy = true;
		error = null;
		try {
			const thread = await unwrap(
				api.POST('/api/v1/items/{key}/comments', {
					params: { path: { key: itemKey } },
					body: { idempotency_key: draftKey(draft, text), body: text }
				})
			);
			// Only if the composer still holds what was sent (it is disabled while posting, so this is a guard, not a race).
			if (body.trim() === text) {
				body = '';
				draft = newDraftKey();
			}
			onposted(thread);
		} catch (err) {
			if (err instanceof Refused) error = err.reasons.map((r) => fullLabel(r)).join('; ');
			else if (err instanceof ApiError) error = err.message;
			else error = errorMessage(err);
		} finally {
			busy = false;
		}
	}
</script>

<ol class="thread">
	{#each comments as c (c.id)}
		<li>
			<span class="muted"
				><span title={c.author}>{short(c.author)}</span> ·
				<span title={new Date(c.at).toLocaleString()}>{age(c.at)}</span></span
			>
			<CommentBody markdown={c.body} />
		</li>
	{:else}
		<li class="muted">No comments yet.</li>
	{/each}
</ol>
<form onsubmit={post}>
	<textarea rows="3" bind:value={body} disabled={busy} placeholder="Add a comment" aria-label="comment on {itemKey}"></textarea>
	<button type="submit" disabled={busy || !body.trim()}>Comment</button>
</form>
{#if error}<p class="error" role="alert">{error}</p>{/if}

<style>
	.thread {
		display: grid;
		gap: 0.5rem;
		margin: 0 0 0.5rem;
		padding: 0;
		list-style: none;
		font-size: 14px;
	}
	form {
		display: flex;
		gap: 0.5rem;
		align-items: flex-start;
	}
	textarea {
		flex: 1;
		font: inherit;
	}
	.muted {
		color: var(--muted);
	}
	.error {
		margin: 0.25rem 0 0;
		color: var(--danger);
	}
</style>
