<script lang="ts">
	/*
	 * One question of the reader's toolbar (direction, page layout, the strip,
	 * zoom) in a track of its own. In one shared pill with hairlines between,
	 * the three questions read as a single bar of five modes.
	 */
	import type { Snippet } from 'svelte';

	let { label, class: className = '', children }: { label: string; class?: string; children: Snippet } = $props();
</script>

<span role="group" aria-label={label} class="flex shrink-0 items-center gap-1 rounded-xl border border-line/60 bg-surface/60 p-0.5 {className}">
	{@render children()}
</span>
