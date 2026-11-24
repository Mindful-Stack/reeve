<!-- web/src/lib/components/GoalRow.svelte: one goal on the sprint page. Closed it is one line: the disclosure with the
     outcome, the confidence dot (active sprint only: a draft takes no confidence, kb/ADR-0026), the initiative tag
     (`initiativeTag`; display only, kb/ADR-0014 R3), what it waits on, its goal links, its progress and the owner's
     initials. Open it adds the initiatives its links reach, the check, the notes, the confidence, the counts (under
     the page's kb/ADR-0014 R2 rule line), the gates, and the page's own controls. It makes no write: the page owns
     which goals are open, so a live refetch keeps them. A warning is carried in words (a visible "!" and hidden text),
     never by colour or hover alone;
     the article is a size container (`container-type: inline-size`), so anything the `controls` snippet positions as
     fixed or absolute is positioned against the row; in a narrow column the link count and the bar drop out of the line. -->
<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { Goal, GoalGates as Gates, GoalProgress } from '#lib/api/types.ts';
	import GoalGates from '#lib/components/GoalGates.svelte';
	import { today } from '#lib/decisions.ts';
	import { dayLabel, plural } from '#lib/format.ts';
	import { confidenceLabel, confidenceTone, initiativeTag, ownerInitials, progressTotal } from '#lib/sprint.ts';

	let {
		goal,
		gates,
		active,
		progress,
		firstBucket,
		open,
		ontoggle,
		lead,
		controls
	}: {
		goal: Goal;
		/** What the goal waits on and the initiatives it reaches; absent while the reads raced an opening. */
		gates: Gates | undefined;
		/** The goal is in the active sprint (not a next-sprint draft): it carries a confidence and counts. */
		active: boolean;
		/** The goal's numbers in the active sprint; absent for a next-sprint goal, and for an active one the counts miss
		 *  (zeros, no bar). */
		progress?: GoalProgress;
		/** The first configured horizon bucket's name, or null: without one no tag warns. */
		firstBucket: string | null;
		open: boolean;
		ontoggle: () => void;
		/** Stands before the disclosure: the next sprint's "Goes in at open" tick box. */
		lead?: Snippet;
		/** The page's controls for this goal, shown when open. */
		controls: Snippet;
	} = $props();

	const tag = $derived(initiativeTag(gates?.initiatives ?? [], firstBucket));
	const waits = $derived((gates?.discoveries.length ?? 0) + (gates?.decisions.length ?? 0));
	const confidenceText = $derived(`confidence: ${goal.confidence ? confidenceLabel(goal.confidence) : 'not recorded'}`);
	const total = $derived(progress ? progressTotal(progress) : 0);
	/** The goal's creation as the browser's local day, in the app's day words ("3 Nov"). */
	const waitingSince = $derived(dayLabel(today(new Date(goal.created_at))));
</script>

<article
	class="goal"
	class:draft={!active}
	data-testid="goal-{goal.id}"
	id="goal-{goal.id}"
	tabindex="-1"
	aria-labelledby="goal-{goal.id}-outcome"
>
	<div class="line">
		{@render lead?.()}
		<button type="button" class="ghost disclose" aria-expanded={open} aria-controls="goal-{goal.id}-detail" onclick={ontoggle}>
			<span class="chevron" aria-hidden="true">{open ? '▾' : '▸'}</span>
			<span class="outcome" class:wrap={open} id="goal-{goal.id}-outcome" title={goal.outcome}>{goal.outcome}</span>
		</button>
		{#if active}
			<span class="dot {confidenceTone(goal.confidence)}" role="img" aria-label={confidenceText} title={confidenceText}></span>
		{/if}
		<span class="tag" class:warn={tag.warn} title={tag.label}
			>
			<span class="tag-text" aria-hidden="true">{tag.text}</span>
			{#if tag.warn}{' '}<span class="mark" aria-hidden="true">!</span>{/if}{' '}
			<span class="visually-hidden">{tag.label}</span>
		</span>
		{#if waits > 0}<span class="waits">waits on {waits}</span>{/if}
		<span class="muted small links">{plural(goal.links.length, 'link', 'links')}</span>
		{#if progress}
			<progress max={Math.max(total, 1)} value={progress.completed} aria-hidden="true"></progress>
			<span class="small numbers"
				><span aria-hidden="true">{progress.completed}/{total}</span>{' '}<span class="visually-hidden"
					>{progress.completed} of {total} completed</span
				></span
			>
		{/if}
		<span class="badge" title={goal.owner}
			><span aria-hidden="true">{ownerInitials(goal.owner)}</span><span class="visually-hidden">Owner: {goal.owner}</span></span
		>
	</div>
	{#if open}
		<div class="detail" id="goal-{goal.id}-detail">
			{#if gates?.initiatives.length}
				<ul class="initiatives" aria-label="Initiatives">
					{#each gates.initiatives as i (i.key)}
						<li>
							<a href="/initiatives/{encodeURIComponent(i.key)}"><span class="key">{i.key}</span> {i.title}</a> ·
							<span class:warn-text={firstBucket !== null && !i.in_first_bucket}
								>{i.horizon}{#if firstBucket !== null && !i.in_first_bucket}{' '}(not in {firstBucket}){/if}</span
							>
						</li>
					{/each}
				</ul>
			{/if}
			<p class="muted">Owner: {goal.owner}</p>
			{#if !active}<p class="muted">Waiting since {waitingSince}</p>{/if}
			<p>We'll know when: {goal.check}</p>
			{#if goal.notes}<p class="notes">{goal.notes}</p>{/if}
			{#if active}
				<p>
					Confidence: <span data-testid="confidence">{goal.confidence ? confidenceLabel(goal.confidence) : 'not recorded'}</span>
					{#if goal.confidence_note}<span class="muted">({goal.confidence_note})</span>{/if}
				</p>
				{#if goal.suggested_confidence}
					<p class="muted" data-testid="suggestion">
						suggested: {confidenceLabel(goal.suggested_confidence)} by {goal.suggested_by}{#if goal.suggested_note}{' '}({goal.suggested_note}){/if}
					</p>
				{/if}
				<p class="numbers">
					{progress?.completed ?? 0} completed · {progress?.in_flight ?? 0} in flight ({progress?.in_review ?? 0} in review) · {progress?.todo ?? 0} todo
				</p>
			{/if}
			{#if gates}<GoalGates {gates} linked={goal.links.length > 0} />{/if}
			{@render controls()}
		</div>
	{/if}
</article>

<style>
	.goal {
		container-type: inline-size;
		border: 1px solid var(--line);
		border-radius: 6px;
		background: var(--raised);
	}
	.goal.draft {
		border-style: dashed;
	}
	.line {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		padding: 4px 8px;
	}
	.disclose {
		display: flex;
		flex: 1;
		align-items: center;
		gap: 6px;
		min-width: 0;
		height: auto;
		padding: 2px 0;
		text-align: left;
	}
	.outcome {
		min-width: 0;
		overflow: hidden;
		font-weight: 600;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.tag,
	.waits,
	.small {
		flex: none;
		font-size: 12px;
		white-space: nowrap;
	}
	.tag {
		display: inline-flex;
		flex: 0 1 auto;
		gap: 4px;
		min-width: 0;
		padding: 1px 6px;
		border: 1px solid var(--line);
		border-radius: 999px;
		color: var(--muted);
		font-family: var(--mono);
	}
	.tag-text {
		min-width: 0;
		max-width: 16ch;
		overflow: hidden;
		text-overflow: ellipsis;
	}
	.mark {
		flex: none;
		font-weight: 700;
	}
	.tag.warn {
		border-color: var(--warn);
		background: var(--warn-soft);
		color: var(--warn);
	}
	.waits,
	.warn-text {
		color: var(--warn);
	}
	.numbers {
		font-variant-numeric: tabular-nums;
	}
	progress {
		flex: none;
		width: 48px;
		height: 6px;
		accent-color: var(--ready);
	}
	.dot {
		flex: none;
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: transparent;
		box-shadow: inset 0 0 0 1px var(--muted);
	}
	.dot.ready {
		background: var(--ready);
		box-shadow: none;
	}
	.dot.warn {
		background: var(--warn);
		box-shadow: none;
	}
	.dot.danger {
		background: var(--danger);
		box-shadow: none;
	}
	.badge {
		display: inline-grid;
		flex: none;
		place-items: center;
		width: 24px;
		height: 24px;
		border-radius: 50%;
		background: var(--selection);
		font-size: 11px;
		font-weight: 600;
	}
	.detail {
		display: grid;
		grid-template-columns: minmax(0, 1fr);
		gap: 4px;
		overflow-wrap: anywhere;
		padding: 4px 12px 10px 28px;
		border-top: 1px solid var(--line);
	}
	.detail p {
		margin: 0;
	}
	.initiatives {
		margin: 0;
		padding: 0;
		list-style: none;
	}
	.outcome.wrap {
		white-space: normal;
		overflow-wrap: anywhere;
	}
	@container (max-width: 600px) {
		.links,
		progress {
			display: none;
		}
	}
	.notes {
		white-space: pre-wrap;
	}
</style>
