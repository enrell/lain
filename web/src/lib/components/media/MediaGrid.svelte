<script lang="ts">
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import MediaCard from './MediaCard.svelte';

	let {
		items,
		progressMap = null,
		priority = false
	}: {
		items: CatalogItem[];
		progressMap?: Map<string, Progress> | null;
		priority?: boolean;
	} = $props();
</script>

<div
	class="grid grid-cols-2 gap-x-3 gap-y-5 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6"
>
	{#each items as item, i (item.id)}
		<MediaCard
			{item}
			enrichment={enrichmentCache.get(item.id) ?? null}
			progress={progressMap?.get(item.id) ?? null}
			priority={priority && i < 6}
		/>
	{/each}
</div>
