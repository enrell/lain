<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import type { SeriesGroup } from '$lib/utilities/grouping';
	import ContinueCard from './ContinueCard.svelte';
	import MediaCard from './MediaCard.svelte';
	import ShowCard from './ShowCard.svelte';
	import { RAIL } from './grid';
	import Rail from './Rail.svelte';

	let {
		title,
		href = null,
		actionLabel = 'View all',
		items = [],
		groups = undefined,
		progressMap = null,
		layout = 'poster'
	}: {
		title: string;
		href?: string | null;
		actionLabel?: string;
		/** Files, one card each (where you are in a show, what is next). */
		items?: CatalogItem[];
		/** Titles instead, one card each in the row's frame, as a library's shelf lists them. */
		groups?: SeriesGroup[];
		progressMap?: Map<string, Progress> | null;
		layout?: 'poster' | 'landscape';
	} = $props();
	const frame = $derived(layout === 'poster' ? ('portrait' as const) : ('landscape' as const));
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
	<!-- Scrollable rail keeps the familiar streaming-library scan pattern (Rail),
	     in the one card size of its frame (RAIL), as every rail and grid has it. -->
	<Rail class={RAIL[frame]}>
		{#each groups ?? [] as group (group.key)}
			<div class="w-[var(--card-w)] shrink-0 snap-start">
				{#if group.count > 1}
					<ShowCard {group} {frame} progressOf={(id) => progressMap?.get(id)} />
				{:else}
					<MediaCard item={group.items[0]} enrichment={enrichmentCache.get(group.items[0].id) ?? null} progress={progressMap?.get(group.items[0].id) ?? null} {frame} />
				{/if}
			</div>
		{/each}
		{#each groups ? [] : items as item (item.id)}
			{#if layout === 'landscape'}
				<div class="w-[var(--card-w)] shrink-0 snap-start">
					<ContinueCard
						{item}
						enrichment={enrichmentCache.get(item.id) ?? null}
						progress={progressMap?.get(item.id) ?? null}
					/>
				</div>
			{:else}
				<div class="w-[var(--card-w)] shrink-0 snap-start">
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
