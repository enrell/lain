<script lang="ts" module>
	/** Grid columns for portrait cards and for landscape ones. */
	export const GRID = 'grid grid-cols-2 gap-x-3 gap-y-6 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6';
	export const GRID_WIDE = 'grid grid-cols-1 gap-x-4 gap-y-6 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4';
</script>

<script lang="ts">
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import MediaCard from './MediaCard.svelte';

	let {
		items,
		progressMap = null,
		priority = false,
		wide = false
	}: {
		items: CatalogItem[];
		progressMap?: Map<string, Progress> | null;
		priority?: boolean;
		wide?: boolean;
	} = $props();
</script>

<div class={wide ? GRID_WIDE : GRID}>
	{#each items as item, i (item.id)}
		<MediaCard
			{item}
			enrichment={enrichmentCache.get(item.id) ?? null}
			progress={progressMap?.get(item.id) ?? null}
			priority={priority && i < 6}
			{wide}
		/>
	{/each}
</div>
