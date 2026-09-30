<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		id,
		title,
		aside,
		children
	}: { id?: string; title: string; aside?: Snippet; children: Snippet } = $props();
</script>

<section {id} class="scroll-mt-28 pt-8 first:pt-0">
	<div class="flex items-center justify-between gap-3 border-b border-hairline pb-2">
		<h2 class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{title}</h2>
		{@render aside?.()}
	</div>
	{@render children()}
</section>
