<!-- The urgent line on an epic or initiative page: the chip, the own mark ("marked by …"), where it is derived from
     ("via story ABC-9 …" as a link) and, for an expired own mark, Re-mark and Clear. Writes belong to the page. -->
<script lang="ts">
	import { page } from '$app/state';
	import type { ItemRef, UrgentMark } from '#lib/api/types.ts';
	import { storyHref } from '#lib/drawer.ts';
	import { daysAgo, urgentLine } from '#lib/urgency.ts';
	import UrgentChip from './UrgentChip.svelte';

	let {
		itemKey,
		mark,
		sources,
		titleOf,
		busy = false,
		onremark,
		onclear
	}: {
		itemKey: string;
		mark: UrgentMark | null;
		sources: ItemRef[];
		/** The title of a source, when the page has it loaded. */
		titleOf: (r: ItemRef) => string | undefined;
		busy?: boolean;
		onremark: () => void;
		onclear: () => void;
	} = $props();

	const now = $derived(new Date());
	const own = $derived(sources.some((s) => s.key === itemKey) ? mark : null);
	const others = $derived(sources.filter((s) => s.key !== itemKey));
	const expired = $derived(mark && !mark.active ? mark : null);
	const ownText = $derived(
		own ? urgentLine(own, [{ key: itemKey, item: sources.find((s) => s.key === itemKey)!.item }], itemKey, now, () => undefined).replace(/^Urgent · /, '') : ''
	);
	const full = $derived(urgentLine(own, sources, itemKey, now, titleOf));

	function href(r: ItemRef): string {
		if (r.item === 'story') return storyHref(page.url, r.key);
		return `/${r.item === 'epic' ? 'epics' : 'initiatives'}/${encodeURIComponent(r.key)}`;
	}
</script>

{#if sources.length > 0}
	<p class="urgent-line" data-testid="urgent-line">
		<UrgentChip line={full} />
		{#if ownText}<span>{ownText}</span>{/if}
		{#if others[0]}
			{@const t = titleOf(others[0])}
			<span
				>via {others[0].item} <a href={href(others[0])}>{others[0].key}</a>{t ? ` ${t}` : ''}{others.length > 1
					? ` and ${others.length - 1} more`
					: ''}</span
			>
		{/if}
	</p>
{/if}
{#if expired}
	<p class="urgent-line expired" data-testid="urgent-expired">
		<span>Urgent mark expired {daysAgo(expired.expires_at, now)}</span>
		<button type="button" class="ghost" aria-label="Re-mark {itemKey}" onclick={onremark} disabled={busy}>Re-mark</button>
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
