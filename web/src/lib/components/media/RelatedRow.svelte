<script lang="ts">
	/*
	 * Where to go from a title: the other titles of its library, as one
	 * rail under the page. Fetched once per title; a failure or an empty
	 * library shows nothing.
	 */
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { Progress } from '$lib/api/types';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { groupItems, type SeriesGroup } from '$lib/utilities/grouping';
	import MediaCard from './MediaCard.svelte';
	import ShowCard from './ShowCard.svelte';

	let { libraryId, exclude, heading }: { libraryId: string; exclude: string[]; heading: string } = $props();

	let groups = $state<SeriesGroup[]>([]);
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
		groups = [];
		if (!id) return;
		void api.catalog
			.list({ libraryId: id, limit: 120 })
			.then(async (page) => {
				const found = groupItems(page.items)
					.filter((g) => !g.items.some((i) => skip.has(i.id)))
					.slice(0, 12);
				await ensureEnrichments(found.map((g) => g.items[0].id));
				if (libraryId === id) groups = found;
			})
			.catch(() => {});
	});
</script>

{#if groups.length > 0}
	<section class="mt-12 space-y-4" aria-label={heading}>
		<h2 class="text-lg font-semibold tracking-[-0.02em] text-foreground">{heading}</h2>
		<!-- Through the gutter to the window's edge, as MediaRow's rails. -->
		<div class="-mx-5 flex snap-x scroll-px-5 gap-3.5 overflow-x-auto px-5 pb-4 no-scrollbar sm:-mx-8 sm:scroll-px-8 sm:px-8 md:gap-4 lg:-mx-10 lg:scroll-px-10 lg:px-10">
			{#each groups as group (group.key)}
				<div class="w-[40vw] max-w-44 shrink-0 snap-start sm:w-40 lg:w-44">
					{#if group.count > 1}
						<ShowCard {group} fit progressOf={(id) => progressMap.get(id)} />
					{:else}
						<MediaCard
							item={group.items[0]}
							enrichment={enrichmentCache.get(group.items[0].id) ?? null}
							progress={progressMap.get(group.items[0].id) ?? null}
							fit
						/>
					{/if}
				</div>
			{/each}
		</div>
	</section>
{/if}
