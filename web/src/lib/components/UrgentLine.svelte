<!-- The urgent line on an epic or initiative page: the chip, the own mark ("marked by …"), where it is derived from
     ("via story ABC-9 …" as a link) and, for an expired own mark, Re-mark and Clear. Writes belong to the page. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { ItemRef, UrgentMark } from '#lib/api/types.ts';
	import { daysAgo, itemRoute, ownMarkText, urgentLine, via } from '#lib/urgency.ts';
	import UrgentChip from './UrgentChip.svelte';

	let {
		itemKey,
		mark,
		sources,
		titleOf,
		now,
		busy = false,
		onremark,
		onclear
	}: {
		itemKey: string;
		mark: UrgentMark | null;
		sources: ItemRef[];
		/** The title of a source, when the page has it loaded. */
		titleOf: (r: ItemRef) => string | undefined;
		/** The page's last load time: the "N d ago" texts count from it. */
		now: Date;
		busy?: boolean;
		onremark: () => void;
		onclear: () => void;
	} = $props();

	const own = $derived(sources.some((s) => s.key === itemKey) ? mark : null);
	const v = $derived(via(sources, itemKey, titleOf));
	const expired = $derived(mark && !mark.active ? mark : null);
	const full = $derived(urgentLine(own, sources, itemKey, now, titleOf));
</script>

{#if sources.length > 0}
	<p class="urgent-line" data-testid="urgent-line">
		<UrgentChip line={full} />
		{#if own}<span>{ownMarkText(own, now)}</span>{/if}
		{#if own && v}<span aria-hidden="true">·</span>{/if}
		{#if v}
			<span
				>via {v.ref.item} <a href={itemRoute(v.ref, page.url)}>{v.ref.key}</a>{v.title ? ` ${v.title}` : ''}{v.more > 0
					? ` and ${v.more} more`
					: ''}</span
			>
		{/if}
	</p>
{/if}
{#if expired}
	<p class="urgent-line expired" data-testid="urgent-expired">
		<span>Urgent mark expired {daysAgo(expired.expires_at, now)}</span>
		<span aria-hidden="true">·</span>
		<button type="button" class="ghost" aria-label="Re-mark {itemKey}" onclick={onremark} disabled={busy}>Re-mark</button>
		<span aria-hidden="true">·</span>
		<button type="button" class="ghost" aria-label="Clear urgent mark on {itemKey}" onclick={onclear} disabled={busy}>Clear</button>
	</p>
{/if}

<style>
	.urgent-line {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 10px;
		margin: 0;
		font-size: 14px;
	}
	.expired {
		color: var(--muted);
	}
</style>
