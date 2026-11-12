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
	import { guessFrame, settleFrame } from '$lib/stores/frames';
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
	// has: guessed when it is drawn, checked once against its pictures and
	// then kept (see `guessFrame`).
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
	// Only the latest load may draw or correct: measuring takes up to a few
	// seconds, so moving on to the next title of the same library could
	// otherwise let the previous title's rail (which lists the new title)
	// land after its own.
	let run = 0;

	async function whole(group: SeriesGroup): Promise<SeriesGroup> {
		try {
			const { items } = await api.catalog.episodes(group.items[0].id);
			return groupItems(items)[0] ?? group;
		} catch {
			return group;
		}
	}

	$effect(() => {
		const id = libraryId;
		const skip = new Set(exclude);
		const newest = recent;
		const given = source;
		const mine = ++run;
		groups = [];
		if (!id && !newest && !given) return;
		const files = given
			? given().then((items) => ({ items, cut: false }))
			: api.catalog
					.list(newest ? { sort: 'recent', limit: 120 } : { libraryId: id, limit: 120 })
					.then((page) => ({ items: page.items, cut: page.total > page.items.length }));
		void files
			.then(async ({ items, cut }) => {
				const all = groupItems(items);
				const listed = (newest ? sortGroups(all, 'recent') : all)
					.filter((g) => !g.items.some((i) => skip.has(i.id)))
					.slice(0, 12);
				// A page cut at 120 files holds only part of a long show: its card
				// counted "1 of 21 watched" where the library says 2 of 26. Each
				// title then gets all of its files, as the library's shelves do.
				const found = cut ? await Promise.all(listed.map(whole)) : listed;
				if (mine !== run) return;
				const guess = await guessFrame(found);
				if (mine !== run) return;
				frame = guess;
				groups = found;
				const settled = await settleFrame(found);
				if (mine === run) frame = settled;
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
