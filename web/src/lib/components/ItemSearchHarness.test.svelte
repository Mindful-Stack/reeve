<!-- Binds `picked` the way the Relations form does, as item ABC-1's form (`exclude`), and shows what was bound. -->
<script lang="ts">
	import type { SearchHit } from '#lib/api/types.ts';
	import ItemSearch from './ItemSearch.svelte';

	let picked = $state<SearchHit | null>(null);
</script>

<ItemSearch label="Other item" bind:picked exclude="ABC-1" delay={0} />
<output data-testid="picked">{picked ? `${picked.key} ${picked.item}` : ''}</output>
