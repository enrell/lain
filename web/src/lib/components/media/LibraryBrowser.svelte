<script lang="ts">
	import { isReadable } from '$lib/reader/kinds';
	import { onMount } from 'svelte';
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import Film from '@lucide/svelte/icons/film';
	import type { CatalogItem, Library, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import MediaGrid, { GRID, GRID_WIDE } from '$lib/components/media/MediaGrid.svelte';
	import ShowCard from '$lib/components/media/ShowCard.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { genreCounts, prefersWide, withGenres } from '$lib/utilities/browse';
	import { errorMessage } from '$lib/utilities/errors';
	import { groupItems, sortGroups } from '$lib/utilities/grouping';

	const PAGE_SIZE = 60;

	/**
	 * Plex-style library: the grid lists shows, not files. A show card
	 * opens the show's own page (/item/{id}), which lists its episodes;
	 * single-file titles (movies) keep the poster grid and link straight
	 * to their item page.
	 */
	let {
		libraries = [],
		libraryId = '',
		showLibraryFilter = false
	}: {
		libraries?: Library[];
		libraryId?: string;
		showLibraryFilter?: boolean;
	} = $props();

	let selectedLibrary = $state('');
	let sort = $state<'title' | 'recent'>('title');
	let items = $state<CatalogItem[]>([]);
	let total = $state(0);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let done = $state(false);
	let sentinel = $state<HTMLElement | null>(null);
	let progressMap = $state<Map<string, Progress>>(new Map());

	async function loadMore(): Promise<void> {
		if (loading || done) return;
		loading = true;
		error = null;
		try {
			const page = await api.catalog.list({
				limit: PAGE_SIZE,
				offset: items.length,
				sort,
				libraryId: selectedLibrary || undefined
			});
			items = [...items, ...page.items];
			total = page.total;
			if (page.items.length === 0 || items.length >= page.total) done = true;
			await ensureEnrichments(page.items.map((i) => i.id));
		} catch (err) {
			error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	async function reset(): Promise<void> {
		picked = [];
		items = [];
		total = 0;
		done = false;
		error = null;
		await loadMore();
	}

	onMount(() => {
		selectedLibrary = libraryId;
		void reset();
		void api.me
			.continueWatching()
			.then((list) => {
				progressMap = new Map(list.map((p) => [p.item_id, p]));
			})
			.catch(() => {});
	});

	// Auto-load when the sentinel approaches the viewport; the button
	// below remains as an explicit, accessible fallback.
	$effect(() => {
		const node = sentinel;
		if (!node) return;
		const observer = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) void loadMore();
			},
			{ rootMargin: '600px 0px' }
		);
		observer.observe(node);
		return () => observer.disconnect();
	});

	const libraryOptions = $derived([
		{ value: '', label: 'All libraries' },
		...libraries.map((lib) => ({ value: lib.id, label: lib.name }))
	]);

	const sortOptions = [
		{ value: 'title', label: 'Title A–Z' },
		{ value: 'recent', label: 'Recently indexed' }
	];

	const enrichmentOf = (id: string) => enrichmentCache.get(id);
	const allGroups = $derived(sortGroups(groupItems(items), sort));
	// Genres narrow what is loaded, so a pick loads the rest of the library
	// (the effect below) instead of filtering one page of it.
	let picked = $state<string[]>([]);
	let moreGenres = $state(false);
	const GENRES_SHOWN = 14;
	const genres = $derived(genreCounts(allGroups, enrichmentOf));
	const shownGenres = $derived(
		moreGenres ? genres : genres.filter((g, i) => i < GENRES_SHOWN || picked.includes(g.genre))
	);
	const groups = $derived(withGenres(allGroups, picked, enrichmentOf));
	const shows = $derived(groups.filter((g) => g.count > 1));
	const singles = $derived(groups.filter((g) => g.count === 1).flatMap((g) => g.items));

	function toggleGenre(genre: string): void {
		picked = picked.includes(genre) ? picked.filter((g) => g !== genre) : [...picked, genre];
	}

	$effect(() => {
		if (picked.length > 0 && !done && !loading && !error) void loadMore();
	});
	const activeType = $derived(
		libraries.find((lib) => lib.id === (selectedLibrary || libraryId))?.type ?? ''
	);
	const reading = $derived(isReadable(activeType));
	// One shape for the whole page, decided by what was loaded first so
	// the grid does not reflow as more arrives.
	const wide = $derived(prefersWide(allGroups.slice(0, PAGE_SIZE), enrichmentOf, reading));
	const showHeading = $derived(activeType === 'movie' ? 'Movies' : reading ? 'Series' : 'Shows');
	const singleHeading = $derived(
		activeType === 'movie' ? 'Movies' : reading ? 'One-shots' : 'Movies & specials'
	);
	const counts = $derived(
		[
			`${total} ${total === 1 ? 'item' : 'items'}`,
			shows.length > 0 ? `${shows.length} ${shows.length === 1 ? (reading ? 'series' : 'show') : reading ? 'series' : 'shows'}` : '',
			singles.length > 0 ? `${singles.length} ${singles.length === 1 ? 'title' : 'titles'}` : '',
			picked.length > 0 ? `${groups.length} matching` : '',
			items.length < total ? (picked.length > 0 ? 'loading the rest…' : `showing ${items.length}`) : ''
		]
			.filter(Boolean)
			.join(' · ')
	);
</script>

<div class="space-y-5">
	<div class="flex flex-wrap items-center justify-between gap-3">
		<p class="text-sm text-muted" aria-live="polite">{counts}</p>
		<div class="flex flex-wrap items-center gap-2">
			{#if showLibraryFilter}
				<div class="w-full min-w-0 sm:w-44">
					<Select
						aria-label="Filter by library"
						bind:value={selectedLibrary}
						options={libraryOptions}
						placeholder="All libraries"
						onValueChange={() => void reset()}
					/>
				</div>
			{/if}
			<div class="w-full min-w-0 sm:w-44">
				<Select
					aria-label="Sort"
					bind:value={sort}
					options={sortOptions}
					onValueChange={() => void reset()}
				/>
			</div>
		</div>
	</div>

	{#if genres.length > 0}
		<div class="flex flex-wrap items-center gap-1.5" role="group" aria-label="Filter by genre">
			{#each shownGenres as g (g.genre)}
				{@const on = picked.includes(g.genre)}
				<button
					type="button"
					aria-pressed={on}
					class={[
						'rounded-full border px-2.5 py-1 text-xs transition-colors',
						on
							? 'border-accent/70 bg-accent/15 text-foreground'
							: 'border-line text-muted hover:border-white/20 hover:text-foreground'
					].join(' ')}
					onclick={() => toggleGenre(g.genre)}
				>
					{g.genre} <span class="font-mono text-[10px] opacity-60">{g.count}</span>
				</button>
			{/each}
			{#if genres.length > GENRES_SHOWN}
				<button
					type="button"
					class="rounded-full px-2.5 py-1 text-xs text-muted hover:text-foreground"
					onclick={() => (moreGenres = !moreGenres)}
				>
					{moreGenres ? 'Fewer' : `${genres.length - shownGenres.length} more`}
				</button>
			{/if}
			{#if picked.length > 0}
				<button
					type="button"
					class="rounded-full px-2.5 py-1 text-xs text-muted hover:text-foreground"
					onclick={() => (picked = [])}
				>
					Clear
				</button>
			{/if}
		</div>
	{/if}

	{#if error && items.length === 0}
		<ErrorState message={error} retry={() => void reset()} />
	{:else if !loading && items.length === 0 && !error}
		<EmptyState
			title="Nothing indexed here"
			description="Once a scan finds files for this library they will appear here."
		>
			{#snippet icon()}<Film class="size-6 text-muted" />{/snippet}
		</EmptyState>
	{:else}
		{#if picked.length > 0 && groups.length === 0 && done}
			<p class="py-10 text-center text-sm text-muted">Nothing carries all of these genres.</p>
		{/if}
		{#if shows.length > 0}
			<section class="space-y-4">
				<h2 class="text-lg font-semibold tracking-[-0.02em] text-foreground">{showHeading}</h2>
				<div class={wide ? GRID_WIDE : GRID}>
					{#each shows as group, i (group.key)}
						<ShowCard
							{group}
							{wide}
							progress={group.items.map((it) => progressMap.get(it.id)).find(Boolean) ?? null}
							priority={i < 6}
						/>
					{/each}
				</div>
			</section>
		{/if}

		{#if singles.length > 0}
			<section class="space-y-4">
				{#if shows.length > 0}
					<h2 class="text-lg font-semibold tracking-[-0.02em] text-foreground">{singleHeading}</h2>
				{/if}
				<MediaGrid items={singles} {progressMap} {wide} />
			</section>
		{/if}

		{#if loading}
			<div class={['mt-6', wide ? GRID_WIDE : GRID].join(' ')}>
				{#each Array(wide ? 4 : 6) as _}
					<Skeleton class={wide ? 'aspect-video' : 'aspect-[2/3]'} />
				{/each}
			</div>
		{:else if !done}
			<div class="mt-8 flex justify-center" bind:this={sentinel}>
				<Button variant="secondary" onclick={() => void loadMore()}>
					<ArrowDown class="size-4" /> Load more
				</Button>
			</div>
		{:else if items.length > 0}
			<p class="mt-8 text-center text-xs text-muted">End of library</p>
		{/if}
		{#if error && items.length > 0}
			<p class="mt-4 text-center text-sm text-danger" role="alert">{error}</p>
		{/if}
	{/if}
</div>
