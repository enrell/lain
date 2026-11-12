<script lang="ts">
	// The Library page's front: one shelf per library, newest first, so
	// collections of different kinds are not poured into one grid. A shelf
	// opens its library, where sorting and genres live.
	import { onMount } from 'svelte';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import { api } from '$lib/api';
	import type { Library, Progress } from '$lib/api/types';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { groupItems, sortGroups, type SeriesGroup } from '$lib/utilities/grouping';
	import MediaCard from './MediaCard.svelte';
	import ShowCard from './ShowCard.svelte';

	const FETCH = 60;
	const SHOWN = 14;

	let { libraries }: { libraries: Library[] } = $props();

	interface Shelf {
		groups: SeriesGroup[];
		total: number;
		failed: boolean;
	}
	let shelves = $state<Record<string, Shelf>>({});
	let progressMap = $state<Map<string, Progress>>(new Map());

	async function load(lib: Library): Promise<void> {
		try {
			const page = await api.catalog.list({ libraryId: lib.id, limit: FETCH, sort: 'recent' });
			const groups = sortGroups(groupItems(page.items), 'recent').slice(0, SHOWN);
			shelves[lib.id] = { groups, total: page.total, failed: false };
			await ensureEnrichments(groups.flatMap((g) => g.items.map((i) => i.id)));
		} catch {
			shelves[lib.id] = { groups: [], total: 0, failed: true };
		}
	}

	onMount(() => {
		for (const lib of libraries) void load(lib);
		void api.me
			.continueWatching()
			.then((list) => (progressMap = new Map(list.map((p) => [p.item_id, p]))))
			.catch(() => {});
	});
</script>

<div class="space-y-10">
	{#each libraries as lib (lib.id)}
		{@const shelf = shelves[lib.id]}
		<section class="space-y-3" aria-labelledby={`shelf-${lib.id}`}>
			<div class="flex items-baseline justify-between gap-4">
				<h2 id={`shelf-${lib.id}`} class="min-w-0">
					<a
						href={`/library/${lib.id}`}
						class="group/title flex items-center gap-1 text-lg font-semibold tracking-[-0.02em] text-foreground hover:text-accent"
					>
						<span class="truncate">{lib.name}</span>
						<ChevronRight class="size-4 shrink-0 text-muted transition-transform group-hover/title:translate-x-0.5" aria-hidden="true" />
					</a>
				</h2>
				{#if shelf && !shelf.failed}
					<a href={`/library/${lib.id}`} class="inline-flex shrink-0 items-center gap-1 text-sm text-foreground/70 hover:text-foreground">
						View all <ChevronRight class="size-3.5" aria-hidden="true" />
					</a>
				{/if}
			</div>
			{#if !shelf}
				<div class="flex gap-4 overflow-hidden">
					{#each Array(6) as _}<Skeleton class="aspect-[2/3] w-40 shrink-0" />{/each}
				</div>
			{:else if shelf.failed}
				<p class="text-sm text-muted">Could not load this library.</p>
			{:else if shelf.groups.length === 0}
				<p class="text-sm text-muted">Nothing indexed here yet.</p>
			{:else}
				<!-- Through the gutter to the window's edge, as MediaRow's rails. -->
				<div class="-mx-5 flex snap-x scroll-px-5 gap-3.5 overflow-x-auto px-5 pb-3 pt-1 no-scrollbar [--card-h:9.5rem] sm:-mx-8 sm:scroll-px-8 sm:px-8 sm:[--card-h:12rem] lg:-mx-10 lg:scroll-px-10 lg:px-10 lg:[--card-h:14rem]">
					{#each shelf.groups as group, i (group.key)}
						<div class="shrink-0 snap-start">
							{#if group.count > 1}
								<ShowCard
									{group}
									fit
									progressOf={(id) => progressMap.get(id)}
									priority={i < 4}
								/>
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
					{#if shelf.total > shelf.groups.reduce((n, g) => n + g.count, 0)}
						<a
							href={`/library/${lib.id}`}
							class="flex h-[var(--card-h)] w-28 shrink-0 snap-start items-center justify-center rounded-xl border border-dashed border-line text-sm text-muted transition-colors hover:border-white/25 hover:text-foreground"
						>
							See all
						</a>
					{/if}
				</div>
			{/if}
		</section>
	{/each}
</div>
