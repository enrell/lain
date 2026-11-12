<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import type { SeriesGroup } from '$lib/utilities/grouping';
	import ContinueCard from './ContinueCard.svelte';
	import MediaCard from './MediaCard.svelte';
	import ShowCard from './ShowCard.svelte';

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
	<!-- Scrollable rail keeps the familiar streaming-library scan pattern. It runs
	     through the page's side padding at every width, so the card that continues
	     is cut by the window, not sliced by an invisible line at the gutter; the
	     first card and the scroll stops stay on the gutter. -->
	<div
		class="-mx-5 flex snap-x scroll-px-5 gap-3.5 overflow-x-auto px-5 pb-4 no-scrollbar sm:-mx-8 sm:scroll-px-8 sm:px-8 md:gap-4 lg:-mx-10 lg:scroll-px-10 lg:px-10 lg:gap-5"
	>
		{#each groups ?? [] as group (group.key)}
			<!-- The sizes of the cards below for the same frame. -->
			<div class={['shrink-0 snap-start', layout === 'landscape' ? 'w-[78vw] max-w-96 sm:w-80 lg:w-[22rem] xl:w-96' : 'w-[40vw] max-w-48 sm:w-40 lg:w-44 xl:w-48'].join(' ')}>
				{#if group.count > 1}
					<ShowCard {group} {frame} progressOf={(id) => progressMap?.get(id)} />
				{:else}
					<MediaCard item={group.items[0]} enrichment={enrichmentCache.get(group.items[0].id) ?? null} progress={progressMap?.get(group.items[0].id) ?? null} {frame} />
				{/if}
			</div>
		{/each}
		{#each groups ? [] : items as item (item.id)}
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
	</div>
</section>
