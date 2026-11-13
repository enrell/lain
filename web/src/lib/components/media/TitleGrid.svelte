<script lang="ts">
	/*
	 * A grid of titles. Titles whose pictures are of one shape share one
	 * frame, at the sizes Home's rails give that shape (see `framedGrid`).
	 * When posters and wide stills mix (every library's titles, a search,
	 * the favourites), each shape is its own part of the grid, in the order
	 * of the set, under a small label, rather than one shape boxed inside
	 * the other's frame: a still letterboxed in a poster's frame lost most
	 * of its card to bars, and rows that mixed both frames read as a jumble.
	 * Arrows move across the parts as across one grid.
	 */
	import type { Progress } from '$lib/api/types';
	import { t } from '$lib/i18n';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import { splitByShape, type Shape } from '$lib/stores/shapes.svelte';
	import { gridKeys } from '$lib/utilities/grid-nav';
	import type { SeriesGroup } from '$lib/utilities/grouping';
	import MediaCard from './MediaCard.svelte';
	import ShowCard from './ShowCard.svelte';
	import { framedGrid } from './grid';

	let {
		groups,
		shapeOf,
		limit = Infinity,
		dense = false,
		progressOf = undefined,
		above = undefined,
		eager = 6,
		cardClass = '',
		id = undefined,
		element = $bindable(null),
		onclick = undefined
	}: {
		/** Every title of the set, in its order. */
		groups: SeriesGroup[];
		/** The shape of a title's picture, held for the set (see `shapesNow`). */
		shapeOf: (group: SeriesGroup) => Shape;
		/** How many cards are drawn, across the parts (the rest come as the page scrolls). */
		limit?: number;
		dense?: boolean;
		progressOf?: (id: string) => Progress | undefined;
		/** Where an up arrow from the first row goes (the field the titles came from). */
		above?: () => HTMLElement | null | undefined;
		/** How many of the first cards load their pictures at once. */
		eager?: number;
		/** Classes for each card's cell (scroll margins under sticky bars). */
		cardClass?: string;
		id?: string;
		element?: HTMLElement | null;
		onclick?: () => void;
	} = $props();

	const parts = $derived(splitByShape(groups, shapeOf));
	// The parts as drawn: `limit` cards in all, the first part filled first.
	const shown = $derived.by(() => {
		let left = limit;
		return parts.map((part) => {
			const items = part.items.slice(0, Math.max(0, left));
			left -= items.length;
			return { ...part, count: part.items.length, items };
		});
	});
	const cardsOf = (grid: HTMLElement) => [...grid.querySelectorAll<HTMLElement>('[data-cards] > *')];
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div {id} bind:this={element} onkeydown={(e) => gridKeys(e, above, cardsOf)} {onclick}>
	{#each shown as part, p (part.shape)}
		{#if part.items.length > 0}
			<!-- The next part under a hairline, as Search's "Or browse" stands apart. -->
			<section class={p > 0 ? 'mt-10 border-t border-line/50 pt-6' : ''} aria-label={parts.length > 1 ? t(part.shape === 'portrait' ? 'browse.partPortrait' : 'browse.partLandscape') : undefined}>
				{#if parts.length > 1}
					<h2 class="mb-4 flex items-baseline gap-2 font-mono text-[10px] uppercase tracking-[0.18em] text-foreground/65">
						{t(part.shape === 'portrait' ? 'browse.partPortrait' : 'browse.partLandscape')}
						<span class="tabular-nums text-muted">· {part.count}</span>
					</h2>
				{/if}
				<div class={[framedGrid(part.shape, dense), cardClass].join(' ')} data-cards>
					{#each part.items as group, i (group.key)}
						{@const first = p === 0 && i < eager}
						{#if group.count > 1}
							<ShowCard {group} frame={part.shape} {progressOf} priority={first} />
						{:else}
							<MediaCard
								item={group.items[0]}
								enrichment={enrichmentCache.get(group.items[0].id) ?? null}
								progress={progressOf?.(group.items[0].id) ?? null}
								priority={first}
								frame={part.shape}
							/>
						{/if}
					{/each}
				</div>
			</section>
		{/if}
	{/each}
</div>
