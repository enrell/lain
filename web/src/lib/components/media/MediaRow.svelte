<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import ContinueCard from './ContinueCard.svelte';
	import MediaCard from './MediaCard.svelte';
	import Rail from './Rail.svelte';

	let {
		title,
		href = null,
		actionLabel = 'View all',
		items,
		progressMap = null,
		layout = 'poster'
	}: {
		title: string;
		href?: string | null;
		actionLabel?: string;
		items: CatalogItem[];
		progressMap?: Map<string, Progress> | null;
		layout?: 'poster' | 'landscape';
	} = $props();
</script>

<section class="space-y-4">
	<div class="flex items-baseline justify-between gap-4">
		<h2 class="section-title flex items-center gap-1">
			{title}
		</h2>
		{#if href}
			<a
				href={href}
				class="inline-flex min-h-6 items-center gap-1 text-sm text-foreground/70 transition-colors hover:text-foreground"
			>
				{actionLabel} <ChevronRight class="size-3.5" aria-hidden="true" />
			</a>
		{/if}
	</div>
	<!-- Scrollable rail keeps the familiar streaming-library scan pattern (Rail). -->
	<Rail class="lg:gap-5">
		{#each items as item (item.id)}
			{#if layout === 'landscape'}
				<div class="w-[78vw] max-w-96 shrink-0 snap-start sm:w-80 lg:w-[22rem] xl:w-96">
					<ContinueCard
						{item}
						enrichment={enrichmentCache.get(item.id) ?? null}
						progress={progressMap?.get(item.id) ?? null}
					/>
				</div>
			{:else}
				<div class="w-[40vw] max-w-48 shrink-0 snap-start sm:w-40 lg:w-44 xl:w-48">
					<MediaCard
						{item}
						enrichment={enrichmentCache.get(item.id) ?? null}
						progress={progressMap?.get(item.id) ?? null}
					/>
				</div>
			{/if}
		{/each}
	</Rail>
</section>
