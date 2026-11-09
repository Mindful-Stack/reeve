<!-- web/src/lib/components/BacklogList.svelte: one ranked list of backlog rows: drag handle, rank, key, title
     (opens the drawer), optional epic, readiness (the server's, kb/ADR-0003 R1), owner and an actions menu. Ranking
     targets come from the rows of this list; the page does the writes. -->
<script lang="ts">
	import type { StoryWithReadiness } from '#lib/api/types.ts';
	import { afterPart, COLUMN_LABEL, COLUMNS, type Column, dropBefore, fullLabel, isUntriaged, neighbourBefore, orderAfterDrop, shortReason } from '#lib/board.ts';
	import { freshNote, live } from '#lib/events.svelte.ts';
	import { session } from '#lib/me.svelte.ts';
	import ActionsMenu, { type MenuItem } from '#lib/ui/ActionsMenu.svelte';
	import Avatar from '#lib/ui/Avatar.svelte';
	import StatusDot from '#lib/ui/StatusDot.svelte';
	import UrgentChip from './UrgentChip.svelte';
	import { urgentLine } from '#lib/urgency.ts';

	let {
		rows,
		full,
		showEpic = false,
		split = false,
		epicTitle,
		href,
		busy = false,
		onrank,
		onmove,
		onepic
	}: {
		rows: StoryWithReadiness[];
		/** The complete todo list in rank order: the server ranks against it, filters only hide rows. */
		full: StoryWithReadiness[];
		/** Show each row's epic (the ranked list and the "Epic not loaded" group). */
		showEpic?: boolean;
		/** "No epic": untriaged stories (the server's reason) and the rest are two runs that rank separately. */
		split?: boolean;
		epicTitle: Map<string, string>;
		/** The drawer link for a story. */
		href: (key: string) => string;
		busy?: boolean;
		/** Place `key` before `before` in the full list (null: at the end). */
		onrank: (key: string, before: string | null) => void;
		/** Move to another column: the page's move path (resolution, refusal and override prompt included). */
		onmove: (item: StoryWithReadiness, to: Column) => void;
		/** Change epic: the page opens the epic dialog. */
		onepic: (item: StoryWithReadiness) => void;
	} = $props();

	/** A drag lives in one list: a drop on another list is not accepted. */
	let dragging = $state<{ item: StoryWithReadiness; peers: StoryWithReadiness[] } | null>(null);

	function peersOf(item: StoryWithReadiness): StoryWithReadiness[] {
		if (!split) return rows;
		return rows.filter((r) => isUntriaged(r) === isUntriaged(item));
	}

	/** Past the run's end a row stays inside its run: before the story that follows the run in the full list. */
	function target(before: string | null, peers: StoryWithReadiness[]): string | null {
		return before ?? afterPart(full, peers);
	}

	function menu(item: StoryWithReadiness): MenuItem[] {
		const peers = peersOf(item);
		const key = item.story.key;
		const up = neighbourBefore(peers, key, 'up');
		const down = neighbourBefore(peers, key, 'down');
		return [
			{ label: 'Move up', onselect: () => up && onrank(key, target(up.before, peers)), disabled: busy || !up },
			{ label: 'Move down', onselect: () => down && onrank(key, target(down.before, peers)), disabled: busy || !down },
			// Every other column, as on a board card.
			...COLUMNS.filter((c) => c !== item.story.delivery).map((c) => ({
				label: `Move to ${COLUMN_LABEL[c]}`,
				onselect: () => onmove(item, c),
				disabled: busy
			})),
			{ label: 'Change epic', onselect: () => onepic(item), disabled: busy }
		];
	}

	function dragStart(e: DragEvent, item: StoryWithReadiness) {
		if (busy) {
			e.preventDefault();
			return;
		}
		dragging = { item, peers: peersOf(item) };
		e.dataTransfer?.setData('text/plain', item.story.key);
		if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
	}

	function dragOver(e: DragEvent) {
		if (!dragging) return;
		e.preventDefault();
		if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
	}

	/** A drop lands where the pointer is among the dragged row's peers (as on the board's todo column); a drop that
	 *  leaves their order unchanged sends nothing. */
	function drop(e: DragEvent) {
		e.preventDefault();
		const d = dragging;
		dragging = null;
		if (!d) return;
		const peers = new Set(d.peers.map((p) => p.story.key));
		const boxes = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLElement>(':scope > li[data-key]')]
			.filter((li) => peers.has(li.dataset.key ?? ''))
			.map((li) => {
				const r = li.getBoundingClientRect();
				return { key: li.dataset.key ?? '', top: r.top, height: r.height };
			});
		const before = dropBefore(e.clientY, boxes);
		if (!orderAfterDrop(d.peers, d.item.story.key, before)) return;
		onrank(d.item.story.key, target(before, d.peers));
	}
</script>

<ol class="rows" aria-busy={busy} ondragover={dragOver} ondrop={drop}>
	{#each rows as r (r.story.key)}
		{@const s = r.story}
		{@const note = freshNote(live.recent, s.key, session.me?.email)}
		<li data-key={s.key} class:fresh={note} class:urgent={s.urgent_sources.length > 0}>
			<!-- The pointer path; the actions menu is the keyboard path, so the handle is out of the tab order. -->
			<button
				type="button"
				class="handle ghost icon"
				tabindex="-1"
				draggable="true"
				aria-label="Drag {s.key} to rank"
				ondragstart={(e) => dragStart(e, r)}
				ondragend={() => (dragging = null)}>⠿</button
			>
			<span class="rank"><span class="visually-hidden">{'rank '}</span>{s.rank}</span>
			<span class="key">{s.key}</span>
			{#if s.urgent_sources.length > 0}
				<UrgentChip
					line={urgentLine(s.urgent_mark, s.urgent_sources, s.key, new Date(), (x) => (x.item === 'epic' && x.key === s.epic && s.epic ? epicTitle.get(s.epic) : undefined))}
				/>
			{/if}
			<a class="title" href={href(s.key)}>{s.title}</a>
			{#if showEpic}<span class="epic">{s.epic ? (epicTitle.get(s.epic) ?? s.epic) : 'No epic'}</span>{/if}
			<span class="end">
				<span title={r.readiness.ready ? undefined : r.readiness.reasons.map((x) => fullLabel(x)).join('\n')}
					><StatusDot tone={r.readiness.ready ? 'ready' : 'warn'} label={shortReason(r.readiness)} /></span
				>
				<Avatar email={s.owner} />
				<ActionsMenu label="Actions for {s.key}" items={menu(r)} control="menu" />
			</span>
			{#if note}<p class="note">{note}</p>{/if}
		</li>
	{/each}
</ol>

<style>
	.rows > li {
		gap: 6px 12px;
		padding: 7px 10px;
	}
	.rows > li.urgent {
		box-shadow: inset 0 0 0 2px var(--danger);
	}
	/* Both rings at once: the urgent ring at the edge, app.css's live-change accent ring just inside it. */
	.rows > li.urgent.fresh {
		box-shadow: inset 0 0 0 2px var(--danger), inset 0 0 0 4px var(--accent);
	}
	.handle {
		cursor: grab;
		color: var(--muted);
	}
	.key,
	.rank {
		color: var(--muted);
		font-size: 13px;
	}
	.rank {
		min-width: 2ch;
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	.title {
		flex: 1 1 200px;
		min-width: 0;
		color: var(--foreground);
		text-decoration: none;
	}
	.title:hover {
		text-decoration: underline;
	}
	.epic {
		max-width: 16rem;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--muted);
		font-size: 13px;
	}
	.end {
		display: flex;
		align-items: center;
		gap: 8px;
		margin-left: auto;
		font-size: 13px;
	}
	.note {
		flex-basis: 100%;
	}
</style>
