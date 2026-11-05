<!-- web/src/lib/ui/Avatar.svelte: an owner as initials; the email is in the title. -->
<script lang="ts">
	import { short } from '#lib/format.ts';

	let { email }: { email: string | null | undefined } = $props();

	const label = $derived(email || 'unowned');
	const initials = $derived.by(() => {
		if (!email) return '–';
		const parts = short(email).split(/[._-]+/).filter(Boolean);
		if (parts.length === 0) return '–';
		const out = parts.length > 1 ? [...parts[0]][0] + [...parts[1]][0] : [...parts[0]].slice(0, 2).join('');
		return out.toUpperCase();
	});
</script>

<span class="avatar" role="img" aria-label={label} title={label}>{initials}</span>

<style>
	.avatar {
		flex: none;
		display: inline-grid;
		place-items: center;
		width: 24px;
		height: 24px;
		border-radius: 50%;
		background: var(--lighter-background);
		color: var(--foreground);
		font-size: 13px;
		font-weight: 600;
	}
</style>
