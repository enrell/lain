<script lang="ts">
	// The Library page's front: one shelf per library, newest first, so
	// collections of different kinds are not poured into one grid. A shelf
	// opens its library, where sorting and genres live.
	import { onMount, type Snippet } from 'svelte';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import { api } from '$lib/api';
	import type { Library, Progress } from '$lib/api/types';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { guessFrame, settleFrame } from '$lib/stores/frames';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import type { Shape } from '$lib/stores/shapes.svelte';
	import { groupItems, sortGroups, type SeriesGroup } from '$lib/utilities/grouping';
	import { t } from '$lib/i18n';
	import MediaCard from './MediaCard.svelte';
	import { RAIL } from './grid';
	import ShowCard from './ShowCard.svelte';

	const FETCH = 60;
	const SHOWN = 14;

	let { libraries, view }: { libraries: Library[]; /** The view switch, kept in reach in the sticky band. */ view?: Snippet } = $props();

	interface Shelf {
		groups: SeriesGroup[];
		total: number;
		failed: boolean;
		/** Guessed when the shelf is drawn, checked once against its pictures, then kept (see `guessFrame`). */
		frame: Shape;
	}
	let shelves = $state<Record<string, Shelf>>({});
	let progressMap = $state<Map<string, Progress>>(new Map());

	/*
	 * The newest files name the titles; each title then gets all of its
	 * files. From the newest 60 alone a show held only its latest episodes:
	 * its card counted 16 of 26 and showed an episode's still where the
	 * library page shows the show's poster.
	 */
	async function whole(group: SeriesGroup): Promise<SeriesGroup> {
		try {
			const { items } = await api.catalog.episodes(group.items[0].id);
			return groupItems(items)[0] ?? group;
		} catch {
			return group;
		}
	}

	async function load(lib: Library): Promise<void> {
		try {
			const page = await api.catalog.list({ libraryId: lib.id, limit: FETCH, sort: 'recent' });
			const recent = sortGroups(groupItems(page.items), 'recent').slice(0, SHOWN);
			const groups = await Promise.all(recent.map(whole));
			// A card needs its picture's overlay only; a long show's every episode
			// would not fit in one request. Drawn without waiting for a picture,
			// then checked once: the frame changes only if the guess was wrong.
			const frame = await guessFrame(groups);
			shelves[lib.id] = { groups, total: page.total, failed: false, frame };
			void settleFrame(groups).then((settled) => {
				if (settled !== frame) shelves[lib.id].frame = settled;
			});
		} catch {
			shelves[lib.id] = { groups: [], total: 0, failed: true, frame: 'portrait' };
		}
	}

	onMount(() => {
		for (const lib of libraries) void load(lib);
		void api.me
			.continueWatching()
			.then((list) => (progressMap = new Map(list.map((p) => [p.item_id, p]))))
			.catch(() => {});
	});

	let scrollY = $state(0);
</script>

<svelte:window bind:scrollY />

<!-- Where each shelf is and the view switch, under the header at every
     scroll position: the switch used to scroll away with the page top. The
     row is as tall as the first row of All titles' bar (LibraryBrowser), so
     the switch stands at one place in both views. -->
<div
	class={[
		'sticky top-12 z-20 -mx-5 mb-6 box-content flex min-h-11 items-center gap-3 bg-background px-5 py-2 transition-shadow sm:-mx-3 sm:min-h-10 sm:px-3 md:top-20',
		scrollY > 40 ? 'shadow-[0_12px_16px_-12px_rgba(0,0,0,0.9)]' : ''
	].join(' ')}
>
	<!-- On a phone the names run past the view switch: the row scrolls, and its
	     end fades so the cut name reads as "more this way", not as missing. -->
	<nav class="-my-2 flex min-w-0 flex-1 items-center gap-1 overflow-x-auto py-2 no-scrollbar max-sm:pe-6 max-sm:[mask-image:linear-gradient(to_right,black_calc(100%_-_2rem),transparent)]" aria-label={t('library.shelves')}>
		{#each libraries as lib (lib.id)}
			<a
				href={`#shelf-${lib.id}`}
				class="shrink-0 rounded-md px-2 py-1.5 font-mono text-[11px] uppercase tracking-[0.14em] text-muted transition-colors first:-ml-2 hover:bg-surface-hover hover:text-foreground"
			>
				{lib.name}
			</a>
		{/each}
	</nav>
	{@render view?.()}
</div>

<div class="space-y-12 md:space-y-14">
	{#each libraries as lib (lib.id)}
		{@const shelf = shelves[lib.id]}
		{@const frame = shelf?.frame ?? 'portrait'}
		<!-- Lands below the header and the band when jumped to. -->
		<section id={`shelf-${lib.id}`} class="scroll-mt-28 space-y-4 md:scroll-mt-40" aria-labelledby={`shelf-title-${lib.id}`}>
			<div class="flex items-baseline justify-between gap-4">
				<h2 id={`shelf-title-${lib.id}`} class="min-w-0">
					<a
						href={`/library/${lib.id}`}
						class="section-title group/title flex items-center gap-1 hover:text-accent"
					>
						<span class="truncate">{lib.name}</span>
						<ChevronRight class="size-4 shrink-0 text-muted transition-transform group-hover/title:translate-x-0.5" aria-hidden="true" />
					</a>
				</h2>
				{#if shelf && !shelf.failed}
					<a href={`/library/${lib.id}`} class="inline-flex min-h-6 shrink-0 items-center gap-1 text-sm text-foreground/70 transition-colors hover:text-foreground">
						{t('library.viewAll')} <ChevronRight class="size-3.5" aria-hidden="true" />
					</a>
				{/if}
			</div>
			{#if !shelf}
				<div class="flex gap-3.5 overflow-hidden md:gap-4 lg:gap-5">
					{#each Array(6) as _}<Skeleton class="aspect-[2/3] w-40 shrink-0 rounded-xl lg:w-44" />{/each}
				</div>
			{:else if shelf.failed}
				<p class="text-sm text-muted">{t('library.shelfFailed')}</p>
			{:else if shelf.groups.length === 0}
				<p class="text-sm text-muted">{t('library.shelfEmpty')}</p>
			{:else}
				<!-- Through the gutter to the window's edge, as MediaRow's rails. -->
				<div
					class={[
						'-mx-5 flex snap-x scroll-px-5 gap-3.5 overflow-x-auto px-5 pb-4 pt-1 no-scrollbar sm:-mx-8 sm:scroll-px-8 sm:px-8 md:gap-4 lg:-mx-10 lg:scroll-px-10 lg:gap-5 lg:px-10',
						RAIL[frame]
					].join(' ')}
				>
					{#each shelf.groups as group, i (group.key)}
						<div class="w-[var(--card-w)] shrink-0 snap-start">
							{#if group.count > 1}
								<ShowCard
									{group}
									{frame}
									progressOf={(id) => progressMap.get(id)}
									priority={i < 4}
								/>
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
					{#if shelf.total > shelf.groups.reduce((n, g) => n + g.count, 0)}
						<a
							href={`/library/${lib.id}`}
							class="flex h-[calc(var(--card-w)/var(--r))] w-28 shrink-0 snap-start items-center justify-center rounded-xl border border-dashed border-line text-sm text-muted transition-colors hover:border-white/25 hover:text-foreground"
						>
							{t('library.seeAll')}
						</a>
					{/if}
				</div>
			{/if}
		</section>
	{/each}
</div>
