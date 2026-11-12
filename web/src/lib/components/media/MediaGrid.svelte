<script lang="ts" module>
	/** Portrait cards in equal columns. */
	export const GRID = 'grid grid-cols-2 gap-x-3 gap-y-6 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6';
	/** Rows of one height; each card is as wide as its picture's shape. */
	export const FIT = 'fit-grid sm:flex sm:flex-wrap gap-x-3.5 gap-y-6 [--card-h:9.5rem] sm:[--card-h:12rem] lg:[--card-h:15rem]';
	/** The same, smaller: more titles on screen. */
	export const FIT_DENSE = 'fit-grid fit-dense sm:flex sm:flex-wrap gap-x-3 gap-y-5 [--card-h:7.5rem] sm:[--card-h:9rem] lg:[--card-h:10.5rem]';

	/*
	 * Equal columns of one frame (see `frameOf`), as many as fit. A tile is
	 * about the size of Home's rail cards for its shape (a poster 10-11rem
	 * wide, a widescreen card 20-22rem), so a poster grid and a widescreen
	 * grid carry the same weight; a phone takes two posters or one wide card
	 * a row, the small tiles three or two. Empty tracks are kept (auto-fill),
	 * so three favourites do not swell to the page's width.
	 */
	const FRAMED = {
		portrait: 'grid grid-cols-2 items-start gap-x-3.5 gap-y-6 sm:grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(10rem,1fr))]',
		landscape: 'grid grid-cols-1 items-start gap-x-3.5 gap-y-6 sm:grid-cols-[repeat(auto-fill,minmax(16rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(19.5rem,1fr))]'
	} as const;
	const FRAMED_DENSE = {
		portrait: 'grid grid-cols-3 items-start gap-x-3 gap-y-5 sm:grid-cols-[repeat(auto-fill,minmax(7rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(7.5rem,1fr))]',
		landscape: 'grid grid-cols-2 items-start gap-x-3 gap-y-5 sm:grid-cols-[repeat(auto-fill,minmax(12rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(14rem,1fr))]'
	} as const;
	export function framedGrid(frame: 'portrait' | 'landscape', dense = false): string {
		return (dense ? FRAMED_DENSE : FRAMED)[frame];
	}

	/*
	 * A rail of one frame sets its cards' width (`--card-w`, for a wrapper
	 * of `w-[var(--card-w)]`) and the frame's ratio (`--r`) by its shape, as
	 * Home's rails are sized: posters 10-12rem wide, widescreen cards
	 * 20-24rem. One height for every rail made a poster a third of a
	 * widescreen card's area.
	 */
	export const RAIL = {
		portrait: '[--card-w:min(40vw,12rem)] [--r:0.6667] sm:[--card-w:10rem] lg:[--card-w:11rem] xl:[--card-w:12rem]',
		landscape: '[--card-w:min(78vw,24rem)] [--r:1.7778] sm:[--card-w:20rem] lg:[--card-w:22rem] xl:[--card-w:24rem]'
	} as const;
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
