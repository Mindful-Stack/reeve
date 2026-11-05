<script lang="ts">
	import { glyph, type Level } from '#lib/glyphs.ts';

	/** The level of the key this sits beside; its name is the accessible text (the glyph itself is hidden from readers). */
	let { level }: { level: Level } = $props();
</script>

<span class="glyph" aria-hidden="true" title={level}>{glyph(level)}</span><span class="level">{`${level} `}</span>

<style>
	.glyph {
		margin-right: 0.25em;
	}
	.level {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip: rect(0 0 0 0);
		white-space: nowrap;
	}
</style>
