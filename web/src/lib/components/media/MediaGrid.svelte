<script lang="ts" module>
	/** Portrait cards in equal columns. */
	export const GRID = 'grid grid-cols-2 gap-x-3 gap-y-6 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6';
	/** Rows of one height; each card is as wide as its picture's shape. */
	export const FIT = 'flex flex-wrap gap-x-3.5 gap-y-6 [--card-h:9.5rem] sm:[--card-h:12rem] lg:[--card-h:15rem]';
	/** The same, smaller: more titles on screen. */
	export const FIT_DENSE = 'flex flex-wrap gap-x-3 gap-y-5 [--card-h:7.5rem] sm:[--card-h:9rem] lg:[--card-h:10.5rem]';
</script>

<script lang="ts">
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import MediaCard from './MediaCard.svelte';

	let {
		items,
		progressMap = null,
		priority = false,
		fit = false
	}: {
		items: CatalogItem[];
		progressMap?: Map<string, Progress> | null;
		priority?: boolean;
		fit?: boolean;
	} = $props();
</script>

<div class={fit ? FIT : GRID}>
	{#each items as item, i (item.id)}
		<MediaCard
			{item}
			enrichment={enrichmentCache.get(item.id) ?? null}
			progress={progressMap?.get(item.id) ?? null}
			priority={priority && i < 6}
			{fit}
		/>
	{/each}
</div>
