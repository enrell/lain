<script lang="ts">
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import MediaCard from './MediaCard.svelte';

	let {
		title,
		href = null,
		items,
		progressMap = null
	}: {
		title: string;
		href?: string | null;
		items: CatalogItem[];
		progressMap?: Map<string, Progress> | null;
	} = $props();
</script>

<section class="space-y-3">
	<div class="flex items-baseline justify-between gap-4">
		<h2 class="text-base font-semibold tracking-tight text-foreground">{title}</h2>
		{#if href}
			<a href={href} class="text-xs font-medium text-muted hover:text-accent">View all</a>
		{/if}
	</div>
	<!-- Scrollable rail on small screens; the same cards work in a grid. -->
	<div class="-mx-4 flex snap-x gap-3 overflow-x-auto px-4 pb-2 no-scrollbar md:-mx-0 md:px-0">
		{#each items as item (item.id)}
			<div class="w-36 shrink-0 snap-start sm:w-40 xl:w-44">
				<MediaCard
					{item}
					enrichment={enrichmentCache.get(item.id) ?? null}
					progress={progressMap?.get(item.id) ?? null}
				/>
			</div>
		{/each}
	</div>
</section>
