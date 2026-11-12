<script lang="ts">
	/*
	 * Where to go from a title: the other titles of its library, as one
	 * rail under the page. Fetched once per title; a failure or an empty
	 * library shows nothing. With `recent`, the newest titles of every
	 * library instead (Search before anything is typed); with `source`,
	 * whatever files it gives, in its order (what friends watch).
	 */
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { settleFrame } from '$lib/stores/frames';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import type { Shape } from '$lib/stores/shapes.svelte';
	import { groupItems, sortGroups, type SeriesGroup } from '$lib/utilities/grouping';
	import MediaCard from './MediaCard.svelte';
	import { RAIL } from './grid';
	import ShowCard from './ShowCard.svelte';

	let {
		libraryId = '',
		exclude = [],
		heading,
		recent = false,
		source = undefined,
		class: className = 'mt-12'
	}: {
		libraryId?: string;
		exclude?: string[];
		heading: string;
		recent?: boolean;
		source?: () => Promise<CatalogItem[]>;
		class?: string;
	} = $props();

	let groups = $state<SeriesGroup[]>([]);
	// One frame for the rail and Home's sizes for it, as the library's shelf
	// has: decided from its pictures before it is drawn (see `settleFrame`).
	let frame = $state<Shape>('portrait');
	// The account's positions, so these cards say watched or under way as the
	// same titles do on the library's shelves; without them a rail of bare
	// posters read as titles never opened.
	let progressMap = $state<Map<string, Progress>>(new Map());
	onMount(() => {
		void api.me
			.continueWatching()
			.then((list) => (progressMap = new Map(list.map((p) => [p.item_id, p]))))
			.catch(() => {});
	});
	$effect(() => {
		const id = libraryId;
		const skip = new Set(exclude);
		const newest = recent;
		const given = source;
		groups = [];
		if (!id && !newest && !given) return;
		const files = given
			? given()
			: api.catalog.list(newest ? { sort: 'recent', limit: 120 } : { libraryId: id, limit: 120 }).then((page) => page.items);
		void files
			.then(async (items) => {
				const all = groupItems(items);
				const found = (newest ? sortGroups(all, 'recent') : all)
					.filter((g) => !g.items.some((i) => skip.has(i.id)))
					.slice(0, 12);
				const settled = await settleFrame(found);
				if (libraryId !== id) return;
				frame = settled;
				groups = found;
			})
			.catch(() => {});
	});
</script>

{#if groups.length > 0}
	<section class={['space-y-4', className].join(' ')} aria-label={heading}>
		<h2 class="text-lg font-semibold tracking-[-0.02em] text-foreground">{heading}</h2>
		<!-- Through the gutter to the window's edge, as MediaRow's rails. -->
		<div class={['-mx-5 flex snap-x scroll-px-5 gap-3.5 overflow-x-auto px-5 pb-4 no-scrollbar sm:-mx-8 sm:scroll-px-8 sm:px-8 md:gap-4 lg:-mx-10 lg:scroll-px-10 lg:px-10', RAIL[frame]].join(' ')}>
			{#each groups as group (group.key)}
				<div class="w-[var(--card-w)] shrink-0 snap-start">
					{#if group.count > 1}
						<ShowCard {group} {frame} progressOf={(id) => progressMap.get(id)} />
					{:else}
						<MediaCard
							item={group.items[0]}
							enrichment={enrichmentCache.get(group.items[0].id) ?? null}
							progress={progressMap.get(group.items[0].id) ?? null}
							{frame}
						/>
					{/if}
				</div>
			{/each}
		</div>
	</section>
{/if}
