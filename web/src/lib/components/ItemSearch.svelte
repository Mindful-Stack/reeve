<!-- web/src/lib/components/ItemSearch.svelte: picks one item of any level by key, external reference or text. The
     Picker (lib/ui/Picker.svelte) owns the list, the keyboard and what a screen reader hears; this asks `GET /search`
     for what is typed and hands it the hits as its options. The server did the matching (a hit may match on its
     brief), so nothing is narrowed here. `picked` is the whole hit: its key and its level. Typing again drops the
     pick: what the box says is then no longer the item that was picked. -->
<script lang="ts">
	import { onDestroy } from 'svelte';
	import { api, unwrap } from '#lib/api/client.ts';
	import type { SearchHit } from '#lib/api/types.ts';
	import { errorMessage } from '#lib/format.ts';
	import { hitLevel } from '#lib/search.ts';
	import Picker from '#lib/ui/Picker.svelte';

	let {
		label,
		placeholder,
		picked = $bindable(null),
		exclude,
		disabled = false,
		delay = 200
	}: {
		label: string;
		/** Shown in the empty search field. */
		placeholder?: string;
		picked?: SearchHit | null;
		/** The key of an item never to offer: the one the link starts from (its own key, not an external reference). */
		exclude?: string;
		disabled?: boolean;
		/** How long typing must pause before the server is asked, in milliseconds. */
		delay?: number;
	} = $props();

	/** Hits offered for one text; the server sends up to 50, the item its key names first. */
	const SHOWN = 8;
	const MORE = `Showing the first ${SHOWN}. Type more to narrow.`;
	let hits = $state.raw<SearchHit[]>([]);
	/** Whether the server answered more hits than are offered. */
	let more = $state(false);
	let value = $state('');
	/** The trimmed text last typed; '' until something is. */
	let typed = $state('');
	let searching = $state(false);
	let failed = $state<string | null>(null);
	let latest = 0;
	let timer: ReturnType<typeof setTimeout> | undefined;

	const options = $derived(hits.map((h) => ({ value: h.key, label: `${h.key} ${h.title} · ${hitLevel(h)}` })));
	const none = $derived(
		typed === '' ? 'Type a key or part of a title' : searching ? 'Searching…' : failed !== null ? `Could not search: ${failed}` : 'No item found'
	);

	/** Asks for `text` once typing pauses. The hits of the text before are dropped at once, and so is the pick (`picked`
	 *  and the Picker's `value`, which would otherwise keep the old item among the options and in the box): Enter never
	 *  picks, and the form never sends, an item that matched something else. An answer that arrives after a newer text
	 *  was typed is dropped too. */
	function ask(text: string) {
		clearTimeout(timer);
		const q = text.trim();
		const seq = ++latest;
		typed = q;
		failed = null;
		hits = [];
		more = false;
		picked = null;
		value = '';
		searching = q !== '';
		if (!q) return;
		timer = setTimeout(async () => {
			try {
				const found = await unwrap(api.GET('/api/v1/search', { params: { query: { q } } }));
				if (seq !== latest) return;
				// The item itself is left out before the cut, so it never takes one of the eight places.
				const others = found.filter((h) => h.key !== exclude);
				more = others.length > SHOWN;
				hits = others.slice(0, SHOWN);
			} catch (e) {
				if (seq !== latest) return;
				failed = errorMessage(e);
			}
			searching = false;
		}, delay);
	}

	onDestroy(() => {
		clearTimeout(timer);
		latest++;
	});
</script>

<Picker
	{label}
	{placeholder}
	{options}
	bind:value
	{disabled}
	{none}
	more={more ? MORE : undefined}
	onquery={ask}
	onchange={(v) => {
		picked = hits.find((h) => h.key === v) ?? null;
	}}
/>
