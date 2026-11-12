<script lang="ts">
	/**
	 * A library as something to search and narrow, not a wall to scroll.
	 *
	 * The whole library is loaded once (titles are small) so that search,
	 * sorting and every filter answer at once and the counts next to each
	 * option are exact. What is set lives in the URL: a view can be
	 * bookmarked, and Back returns to it. The grid lists titles, one card
	 * per show (it opens the show's page) or per single file, all in one
	 * frame (see `settleFrame`).
	 *
	 * The titles are a library's, every library's, or whatever `source`
	 * gives (the favourites), so every list of titles browses the same way.
	 */
	import { onMount, tick, untrack, type Snippet } from 'svelte';
	import { afterNavigate, beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import Grid2x2 from '@lucide/svelte/icons/grid-2x2';
	import Grid3x3 from '@lucide/svelte/icons/grid-3x3';
	import Film from '@lucide/svelte/icons/film';
	import Search from '@lucide/svelte/icons/search';
	import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
	import X from '@lucide/svelte/icons/x';
	import type { CatalogItem, Library, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { framedGrid } from '$lib/components/media/grid';
	import MediaCard from '$lib/components/media/MediaCard.svelte';
	import ShowCard from '$lib/components/media/ShowCard.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { settleFrame } from '$lib/stores/frames';
	import { sampleArt, type Shape } from '$lib/stores/shapes.svelte';
	import {
		ADDED, LISTED, NO_FILTERS, SHAPES, SORTS, WATCH,
		activeCount, anyCount, apply, countBy, filtersFromParams, filtersToParams, genreFacets, watchState,
		type Context, type Filters, type Sort
	} from '$lib/utilities/browse';
	import { errorMessage } from '$lib/utilities/errors';
	import { gridKeys } from '$lib/utilities/grid-nav';
	import { groupItems } from '$lib/utilities/grouping';
	import { t, type Key } from '$lib/i18n';

	const FETCH = 500;
	const ENRICH = 150;
	const RENDER = 96;
	const GENRES_SHOWN = 24;

	let {
		libraries = [],
		libraryId = '',
		showLibraryFilter = false,
		name = '',
		backHref = '',
		header,
		view,
		empty,
		source,
		listed = '',
		prefsKey = 'lain.library.prefs',
		searchLabel = ''
	}: {
		libraries?: Library[];
		libraryId?: string;
		showLibraryFilter?: boolean;
		/** Shown in the sticky bar once the page header has scrolled away. */
		name?: string;
		backHref?: string;
		/**
		 * The page's own heading. On a wide screen it heads the filter column,
		 * so the search bar starts level with it; above the grid it left a band
		 * of nothing over the toolbar.
		 */
		header?: Snippet;
		/** A control kept at the right end of the sticky bar (the Library page's view switch). */
		view?: Snippet;
		/** Shown instead of the generic empty state when there are no titles at all. */
		empty?: Snippet;
		/** The titles' files, instead of a library's or every library's. */
		source?: () => Promise<CatalogItem[]>;
		/** The label of the order `source` gives them in; set, that order is offered first and is the default. */
		listed?: string;
		/** Where this browser keeps its sort and tile size. */
		prefsKey?: string;
		searchLabel?: string;
	} = $props();

	let selectedLibrary = $state('');
	let items = $state<CatalogItem[]>([]);
	let total = $state(0);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let progressMap = $state<Map<string, Progress>>(new Map());
	let now = $state(Math.floor(Date.now() / 1000));
	let run = 0;

	// Per-browser conveniences: the sort used last and the tile size.
	let defaultSort = $state<Sort>('title');
	let dense = $state(false);
	function savePrefs(): void {
		try {
			localStorage.setItem(prefsKey, JSON.stringify({ sort: defaultSort, dense }));
		} catch {
			/* private window: lasts for this visit */
		}
	}

	// ---- where you were ----------------------------------------------------
	// Opening a title and coming Back returns to the same spot: the scroll
	// position and how much of the grid was rendered are kept per URL.
	const PLACE = 'lain.library.place:';
	let scrollY = $state(0);
	let returning = false;
	const placeKey = () => PLACE + page.url.pathname + page.url.search;
	beforeNavigate(() => {
		try {
			sessionStorage.setItem(placeKey(), JSON.stringify({ y: window.scrollY, shown }));
		} catch {
			/* no storage: Back lands at the top */
		}
	});
	afterNavigate((nav) => {
		returning = nav.type === 'popstate';
	});
	async function restorePlace(): Promise<void> {
		if (!returning) return;
		returning = false;
		try {
			const saved = JSON.parse(sessionStorage.getItem(placeKey()) ?? 'null');
			if (!saved || !(saved.y > 0)) return;
			shown = Math.max(RENDER, Number(saved.shown) || RENDER);
			await tick();
			window.scrollTo({ top: saved.y });
		} catch {
			/* stay at the top */
		}
	}

	function toTop(): void {
		const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		window.scrollTo({ top: 0, behavior: calm ? 'auto' : 'smooth' });
	}
	let gridTop = $state<HTMLElement | null>(null);
	let gridEl = $state<HTMLElement | null>(null);
	/** A new result set starts at its beginning, not mid-scroll. */
	function toResults(): void {
		if (!gridTop) return;
		const top = gridTop.getBoundingClientRect().top + window.scrollY - (window.innerWidth < 768 ? 150 : 190);
		if (window.scrollY > top) window.scrollTo({ top: Math.max(0, top) });
	}

	async function load(): Promise<void> {
		const mine = ++run;
		items = [];
		held = null;
		total = 0;
		error = null;
		loading = true;
		try {
			if (source) {
				const got = await source();
				if (mine !== run) return;
				items = got;
				total = got.length;
			} else {
				for (;;) {
					const got = await api.catalog.list({
						limit: FETCH,
						offset: items.length,
						sort: 'title',
						libraryId: selectedLibrary || undefined
					});
					if (mine !== run) return;
					items = [...items, ...got.items];
					total = got.total;
					if (got.items.length === 0 || items.length >= got.total) break;
				}
			}
			loading = false;
			void restorePlace();
			// Overlays carry the genres and artwork; the titles are usable
			// before they arrive.
			const ids = items.map((i) => i.id);
			for (let at = 0; at < ids.length && mine === run; at += ENRICH) {
				await ensureEnrichments(ids.slice(at, at + ENRICH));
			}
		} catch (err) {
			if (mine === run) error = errorMessage(err);
		} finally {
			if (mine === run) loading = false;
		}
	}

	onMount(() => {
		if (listed) defaultSort = LISTED;
		try {
			const saved = JSON.parse(localStorage.getItem(prefsKey) ?? '{}');
			if (sortOptions.some((s) => s.value === saved.sort)) defaultSort = saved.sort;
			dense = saved.dense === true;
		} catch {
			/* defaults */
		}
		selectedLibrary = libraryId;
		void load();
		void api.me
			.continueWatching()
			.then((list) => (progressMap = new Map(list.map((p) => [p.item_id, p]))))
			.catch(() => {});
	});

	// ---- what is set: the URL ----------------------------------------------

	const sortOptions = $derived<{ value: Sort; label: string }[]>([
		...(listed ? [{ value: LISTED as Sort, label: listed }] : []),
		...SORTS.map((s) => ({ value: s.value as Sort, label: s.label as string }))
	]);
	const filters = $derived(
		filtersFromParams(
			page.url.searchParams,
			defaultSort,
			sortOptions.map((s) => s.value)
		)
	);
	const active = $derived(activeCount(filters));

	function set(patch: Partial<Filters>): void {
		const query = filtersToParams({ ...filters, ...patch }, defaultSort).toString();
		shown = RENDER;
		toResults();
		void goto(query ? `?${query}` : page.url.pathname, { replaceState: true, keepFocus: true, noScroll: true });
	}
	const toggle = <K extends 'watch' | 'added' | 'shape' | 'kind'>(key: K, value: Filters[K]) =>
		set({ [key]: filters[key] === value ? NO_FILTERS[key] : value } as Partial<Filters>);

	const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
	function genre(value: string, mode: 'want' | 'not'): void {
		const wanted = filters.genres.some((g) => same(g, value));
		const excluded = filters.exclude.some((g) => same(g, value));
		const genres = filters.genres.filter((g) => !same(g, value));
		const exclude = filters.exclude.filter((g) => !same(g, value));
		if (mode === 'want' && !wanted) genres.push(value);
		if (mode === 'not' && !excluded) exclude.push(value);
		set({ genres, exclude });
	}

	// The search box is typed into faster than the URL should change.
	let text = $state('');
	let typed = '';
	let timer: ReturnType<typeof setTimeout> | undefined;
	let searchBox = $state<HTMLInputElement | null>(null);
	$effect(() => {
		// Back, or a cleared filter set: the URL is the truth.
		if (filters.q !== typed) {
			typed = filters.q;
			text = filters.q;
		}
	});
	function onSearch(value: string): void {
		text = value;
		clearTimeout(timer);
		timer = setTimeout(() => {
			typed = value.trim();
			set({ q: value });
		}, 140);
	}

	// ---- what it gives ------------------------------------------------------

	const ctx = $derived<Context>({
		enrichment: (id) => enrichmentCache.get(id),
		progress: (id) => progressMap.get(id),
		now
	});
	const all = $derived(groupItems(items));
	const results = $derived(apply(all, filters, ctx));
	let shown = $state(RENDER);
	const visible = $derived(results.slice(0, shown));
	// One frame for the whole grid, from its first titles' pictures, measured
	// before the grid is drawn and then held (see `settleFrame`): it opened
	// in one shape and turned to the other as the first posters came in. A
	// later set (a filter, a search) is drawn at once in the frame held, and
	// takes its own once its pictures are measured.
	let held = $state<{ key: string; frame: Shape } | null>(null);
	const sampleKey = $derived(sampleArt(results).map((i) => i.id).join(' '));
	const frame = $derived<Shape>(held?.frame ?? 'portrait');
	let settling = 0;
	$effect(() => {
		const key = sampleKey;
		if ((loading && items.length === 0) || held?.key === key) return;
		const mine = ++settling;
		void settleFrame(untrack(() => results)).then((f) => {
			if (mine === settling) held = { key, frame: f };
		});
	});
	// Loaded, and nothing at all: no tools for an empty list.
	const bare = $derived(!loading && !error && all.length === 0);
	let sentinel = $state<HTMLElement | null>(null);
	$effect(() => {
		const node = sentinel;
		if (!node) return;
		const observer = new IntersectionObserver(
			(entries) => {
				if (entries.some((e) => e.isIntersecting)) shown += RENDER;
			},
			{ rootMargin: '900px 0px' }
		);
		observer.observe(node);
		return () => observer.disconnect();
	});

	// Options with what picking each would leave.
	const genres = $derived(genreFacets(all, filters, ctx));
	let genreQuery = $state('');
	let allGenres = $state(false);
	const genreRows = $derived.by(() => {
		const q = genreQuery.trim().toLowerCase();
		const list = q ? genres.filter((g) => g.value.toLowerCase().includes(q)) : genres;
		return q || allGenres ? list : list.slice(0, GENRES_SHOWN);
	});
	const watchCounts = $derived(countBy(all, filters, ctx, 'watch', (g) => [watchState(g, ctx.progress)]));
	const shapeCounts = $derived(countBy(all, filters, ctx, 'shape', (g) => [g.count > 1 ? 'series' : 'single']));
	const kindCounts = $derived(countBy(all, filters, ctx, 'kind', (g) => [...new Set(g.items.map((i) => i.kind))]));
	const kinds = $derived([...new Set(items.map((i) => i.kind))].filter(Boolean).sort());
	// A kind by name; one the catalog adds later shows as stored until it has one.
	const KIND_NAMES: Record<string, Key> = { video: 'browse.kind.video', episode: 'browse.kind.episode', manga: 'browse.kind.manga', comic: 'browse.kind.comic' };
	const kindName = (k: string) => (KIND_NAMES[k] ? t(KIND_NAMES[k]) : k);
	const addedCounts = $derived(
		new Map(ADDED.map((a) => [a.value, all.filter((g) => apply([g], { ...filters, added: a.value }, ctx).length > 0).length]))
	);

	const chips = $derived([
		...filters.genres.map((g) => ({ label: g, off: () => genre(g, 'want') })),
		...filters.exclude.map((g) => ({ label: `not ${g}`, off: () => genre(g, 'not') })),
		...(filters.watch ? [{ label: WATCH.find((w) => w.value === filters.watch)!.label, off: () => set({ watch: '' }) }] : []),
		...(filters.shape ? [{ label: SHAPES.find((s) => s.value === filters.shape)!.label, off: () => set({ shape: '' }) }] : []),
		...(filters.kind ? [{ label: kindName(filters.kind), off: () => set({ kind: '' }) }] : []),
		...(filters.added ? [{ label: `Added: ${ADDED.find((a) => a.value === filters.added)!.label.toLowerCase()}`, off: () => set({ added: 0 }) }] : [])
	]);
	const narrowed = $derived(active > 0 || filters.q.trim() !== '');
	const clearAll = () => set({ ...NO_FILTERS, sort: filters.sort });

	const libraryOptions = $derived([
		{ value: '', label: 'All libraries' },
		...libraries.map((lib) => ({ value: lib.id, label: lib.name }))
	]);
	let sortValue = $state<string>('title');
	$effect(() => {
		sortValue = filters.sort;
	});

	let sheet = $state(false);
	function pickSort(next: Sort): void {
		// Ask for it while it is not yet the default, so the URL keeps
		// it, then remember it for next time.
		set({ sort: next });
		defaultSort = next;
		savePrefs();
	}

	function onKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const target = e.target as HTMLElement | null;
		const typing = target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName));
		if (e.key === 'Escape' && target === searchBox && text) {
			e.preventDefault();
			onSearch('');
			return;
		}
		if (typing || document.querySelector('[role="dialog"]')) return;
		if (e.key === '/') {
			e.preventDefault();
			searchBox?.focus();
			searchBox?.select();
		} else if (e.key === 'f') {
			e.preventDefault();
			if (window.matchMedia('(min-width: 1024px)').matches) document.getElementById('genre-find')?.focus();
			else sheet = true;
		} else if (e.key === 'c' && narrowed) {
			e.preventDefault();
			clearAll();
		} else if (e.key === 't' && scrollY > 400) {
			e.preventDefault();
			toTop();
		} else if (e.key === 'Backspace' && backHref) {
			e.preventDefault();
			void goto(backHref);
		}
	}
</script>

<svelte:window onkeydown={onKey} bind:scrollY />

{#snippet option(label: string, count: number | undefined, on: boolean, pick: () => void)}
	<button
		type="button"
		aria-pressed={on}
		disabled={!on && !count}
		class={[
			// As wide as a genre's row beside its hide button (44 px, 28 on a wide screen,
			// plus the gap), so every count in the panel stands in one column.
			'flex min-h-11 w-[calc(100%-2.875rem)] items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-sm transition-colors disabled:opacity-35 lg:min-h-0 lg:w-[calc(100%-1.875rem)]',
			on ? 'bg-accent/15 text-foreground' : 'text-muted hover:bg-surface-hover hover:text-foreground'
		].join(' ')}
		onclick={pick}
	>
		<!-- A ring that fills: these are choices of one, and say so before they are touched. -->
		<span class="flex min-w-0 items-center gap-2.5">
			<span class={['size-3.5 shrink-0 rounded-full border', on ? 'border-accent bg-accent shadow-[inset_0_0_0_2.5px_var(--color-background)]' : 'border-foreground/35'].join(' ')} aria-hidden="true"></span>
			<span class="truncate">{label}</span>
		</span>
		<span class="font-mono text-[11px] tabular-nums opacity-70">{count ?? 0}</span>
	</button>
{/snippet}

{#snippet filtersButton(place: string)}
	<button
		type="button"
		class={['h-11 shrink-0 items-center gap-2 rounded-md border px-3.5 text-sm sm:h-10', place, active > 0 ? 'border-accent/60 text-foreground' : 'border-line text-muted'].join(' ')}
		aria-label={active > 0 ? `Filters (${active} active)` : 'Filters'}
		onclick={() => (sheet = true)}
	>
		<SlidersHorizontal class="size-4" />
		<!-- Named, as in the app: the sliders alone did not say "filters". Below a
		     wide screen the sheet also holds the sort. -->
		Sort &amp; filter{#if active > 0}<span class="font-mono text-xs tabular-nums">· {active}</span>{/if}
	</button>
{/snippet}

{#snippet heading(label: string)}
	<h3 class="mb-1 px-2 font-mono text-[10px] uppercase tracking-[0.16em] text-muted/80">{label}</h3>
{/snippet}

{#snippet panel(findId: string, sheet: boolean)}
	<div class="space-y-5">
		{#if sheet}
			<div>
				{@render heading('Sort')}
				<div class="grid grid-cols-2 gap-1.5 px-2">
					{#each sortOptions as s (s.value)}
						<button
							type="button"
							aria-pressed={filters.sort === s.value}
							class={['min-h-11 rounded-md border px-3 text-left text-sm', filters.sort === s.value ? 'border-accent/60 bg-accent/15 text-foreground' : 'border-line text-muted'].join(' ')}
							onclick={() => pickSort(s.value)}
						>
							{s.label}
						</button>
					{/each}
				</div>
			</div>
			<div>
				{@render heading('Tiles')}
				<div class="grid grid-cols-2 gap-1.5 px-2">
					{#each [[false, 'Large'], [true, 'Small']] as const as [value, label] (label)}
						<button
							type="button"
							aria-pressed={dense === value}
							class={['min-h-11 rounded-md border px-3 text-left text-sm', dense === value ? 'border-accent/60 bg-accent/15 text-foreground' : 'border-line text-muted'].join(' ')}
							onclick={() => {
								dense = value;
								savePrefs();
							}}
						>
							{label}
						</button>
					{/each}
				</div>
			</div>
		{/if}
		{#if showLibraryFilter}
			<div>
				{@render heading('Library')}
				<Select
					aria-label="Library"
					bind:value={selectedLibrary}
					options={libraryOptions}
					placeholder="All libraries"
					onValueChange={() => {
						clearAll();
						void load();
					}}
				/>
			</div>
		{/if}
		<!-- Each choice of one starts with "Any": a ring that was picked had no
		     visible way back to everything short of clearing every filter. -->
		<div>
			{@render heading('Progress')}
			{@render option(t('browse.any'), anyCount(all, filters, ctx, 'watch'), !filters.watch, () => set({ watch: '' }))}
			{#each WATCH as w (w.value)}
				{@render option(w.label, watchCounts.get(w.value), filters.watch === w.value, () => toggle('watch', w.value))}
			{/each}
		</div>
		{#if (shapeCounts.get('series') ?? 0) > 0 && (shapeCounts.get('single') ?? 0) > 0}
			<div>
				{@render heading('Shape')}
				{@render option(t('browse.any'), anyCount(all, filters, ctx, 'shape'), !filters.shape, () => set({ shape: '' }))}
				{#each SHAPES as s (s.value)}
					{@render option(s.label, shapeCounts.get(s.value), filters.shape === s.value, () => toggle('shape', s.value))}
				{/each}
			</div>
		{/if}
		{#if kinds.length > 1}
			<div>
				{@render heading('Kind')}
				{@render option(t('browse.any'), anyCount(all, filters, ctx, 'kind'), !filters.kind, () => set({ kind: '' }))}
				{#each kinds as k (k)}
					{@render option(kindName(k), kindCounts.get(k), filters.kind === k, () => toggle('kind', k))}
				{/each}
			</div>
		{/if}
		<!-- "Added" is offered when it tells titles apart: four rows that all
		     say the whole library are noise. -->
		{#if filters.added || ADDED.some((a) => addedCounts.get(a.value) !== all.length)}
			<div>
				{@render heading('Added')}
				{@render option(t('browse.any'), anyCount(all, filters, ctx, 'added'), !filters.added, () => set({ added: 0 }))}
				{#each ADDED as a (a.value)}
					{@render option(a.label, addedCounts.get(a.value), filters.added === a.value, () => toggle('added', a.value))}
				{/each}
			</div>
		{/if}
		{#if genres.length > 0}
			<div>
				{@render heading(`Genres · ${genres.length}`)}
				{#if genres.length > 8}
					<input
						id={findId}
						type="search"
						bind:value={genreQuery}
						placeholder="Find a genre"
						aria-label="Find a genre"
						autocomplete="off"
						spellcheck="false"
						class="mb-1.5 h-8 w-full rounded-md border border-line bg-field px-2 text-sm text-foreground placeholder:text-muted/70 focus:border-accent/60 focus:outline-none"
					/>
				{/if}
				<ul>
					{#each genreRows as g (g.value)}
						{@const want = filters.genres.some((x) => same(x, g.value))}
						{@const not = filters.exclude.some((x) => same(x, g.value))}
						<li class="group/row flex items-center gap-0.5">
							<button
								type="button"
								aria-pressed={want}
								class={[
									'flex min-h-11 min-w-0 flex-1 items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-sm transition-colors lg:min-h-0',
									want ? 'bg-accent/15 text-foreground' : not ? 'text-muted/60 line-through' : 'text-muted hover:bg-surface-hover hover:text-foreground'
								].join(' ')}
								onclick={() => genre(g.value, 'want')}
							>
								<!-- A square that fills: several genres can be picked together. -->
								<span class="flex min-w-0 items-center gap-2.5">
									<span class={['size-3.5 shrink-0 rounded-[4px] border', want ? 'border-accent bg-accent' : 'border-foreground/35'].join(' ')} aria-hidden="true"></span>
									<span class="truncate">{g.value}</span>
								</span>
								<span class="font-mono text-[11px] tabular-nums opacity-70">{g.count}</span>
							</button>
							<button
								type="button"
								aria-pressed={not}
								aria-label={not ? `Stop hiding ${g.value}` : `Hide ${g.value}`}
								title={not ? 'Stop hiding' : 'Hide titles with this genre'}
								class={[
									'flex size-11 shrink-0 items-center justify-center rounded-md text-base transition-opacity hover:bg-surface-hover hover:text-foreground lg:size-7 lg:text-sm',
									not ? 'text-danger opacity-100' : 'text-muted opacity-60 focus-visible:opacity-100 lg:opacity-0 lg:group-hover/row:opacity-100'
								].join(' ')}
								onclick={() => genre(g.value, 'not')}
							>
								−
							</button>
						</li>
					{/each}
				</ul>
				{#if genreRows.length === 0}
					<p class="px-2 py-1.5 text-sm text-muted">No genre like that.</p>
				{:else if !genreQuery.trim() && genres.length > GENRES_SHOWN}
					<button type="button" class="mt-1 px-2 py-1 text-xs text-muted hover:text-foreground" onclick={() => (allGenres = !allGenres)}>
						{allGenres ? 'Show fewer' : `Show all ${genres.length}`}
					</button>
				{/if}
			</div>
		{/if}
	</div>
{/snippet}

<!-- Two rows on a wide screen: the heading then the filters on the left, the
     toolbar and grid spanning both on the right. The heading's row is sized
     by the heading alone (the grid spans the flexible row). -->
<div class="lg:grid lg:grid-cols-[14.5rem_minmax(0,1fr)] lg:grid-rows-[auto_1fr] lg:gap-x-8">
	{#if header}
		<div class="mb-4 lg:col-start-1 lg:row-start-1 lg:mb-6">{@render header()}</div>
	{/if}
	<aside class={['hidden lg:col-start-1 lg:row-start-2', bare ? '' : 'lg:block'].join(' ')} aria-label="Filters">
		<div class="sticky top-24 -ml-2 max-h-[calc(100vh-7rem)] overflow-y-auto pb-6 pr-1">
			{@render panel('genre-find', false)}
		</div>
	</aside>

	<div class="min-w-0 space-y-4 lg:col-start-2 lg:row-span-2 lg:row-start-1">
		<!-- Stuck right under the header (h-20), so no top padding is needed to clear
		     it: at rest the bar starts level with the heading. Its shadow shows only
		     once the grid scrolls under it; at rest it drew a dark seam on the page. -->
		{#if !bare || view}
			<div class={['sticky top-12 z-20 -mx-5 space-y-2 bg-background px-5 pb-2 pt-2 transition-shadow sm:-mx-3 sm:space-y-3 sm:px-3 sm:pb-3 md:top-20', scrollY > 40 ? 'shadow-[0_12px_16px_-12px_rgba(0,0,0,0.9)]' : ''].join(' ')}>
				<!-- The first row is as tall as the shelves' band (LibraryShelves), and the
				     view switch ends it in both: switching views leaves it where it was.
				     It sat a row lower here, under the search. -->
				<div class="flex min-h-11 items-center gap-2 sm:min-h-10">
					{#if !bare}
						<div class="relative min-w-0 flex-1">
							<Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted" aria-hidden="true" />
							<input
								bind:this={searchBox}
								type="search"
								value={text}
								oninput={(e) => onSearch(e.currentTarget.value)}
								onkeydown={(e) => {
									// Down from the field walks into the titles, as from the header's search.
									if (e.key !== 'ArrowDown' || e.altKey || e.ctrlKey || e.metaKey) return;
									const first = gridEl?.querySelector<HTMLElement>('a[href]');
									if (!first) return;
									e.preventDefault();
									first.focus();
								}}
								placeholder={searchLabel || 'Search this library'}
								aria-label={searchLabel || 'Search this library'}
								autocomplete="off"
								spellcheck="false"
								enterkeyhint="search"
								class="h-11 w-full text-ellipsis rounded-md border border-line bg-field pl-9 pr-10 text-sm sm:h-10 md:pr-12 text-foreground placeholder:text-muted/70 transition-colors hover:border-white/15 focus:border-accent/60 focus:outline-none [&::-webkit-search-cancel-button]:hidden"
							/>
							{#if text}
								<button type="button" aria-label="Clear search" class="absolute right-1.5 top-1/2 flex size-7 -translate-y-1/2 items-center justify-center rounded-md text-muted hover:bg-surface-hover hover:text-foreground" onclick={() => { onSearch(''); searchBox?.focus(); }}>
									<X class="size-4" />
								</button>
							{:else}
								<kbd class="pointer-events-none absolute right-3 top-1/2 hidden -translate-y-1/2 text-muted/70 md:inline-flex">/</kbd>
							{/if}
						</div>
						<!-- With the view switch beside them, sort and tile size stay in the
						     sheet until a wide screen has room for all three. -->
						<div class={['hidden w-44 shrink-0', view ? 'lg:block' : 'sm:block'].join(' ')}>
							<Select
								aria-label="Sort"
								bind:value={sortValue}
								options={sortOptions}
								onValueChange={() => pickSort(sortValue as Sort)}
							/>
						</div>
						<button
							type="button"
							class={['hidden h-10 shrink-0 items-center gap-2 rounded-md border border-line px-3 text-sm text-muted transition-colors hover:border-white/15 hover:text-foreground', view ? 'lg:flex' : 'sm:flex'].join(' ')}
							aria-pressed={dense}
							aria-label={dense ? 'Tile size: small. Switch to large.' : 'Tile size: large. Switch to small.'}
							title="Tile size"
							onclick={() => {
								dense = !dense;
								savePrefs();
							}}
						>
							<!-- An icon of what it changes beside the word: a bare "LARGE" read as a label. -->
							{#if dense}<Grid3x3 class="size-4" aria-hidden="true" />{:else}<Grid2x2 class="size-4" aria-hidden="true" />{/if}
							<!-- Says what is set, as the sheet's "Tiles" does: "Large tiles" read as the size it would switch to. -->
							<!-- Beside the view switch on a laptop's width the icon alone says it; the search keeps the room. -->
							<span class={view ? 'hidden xl:inline' : ''}>{t('browse.tiles', { size: t(dense ? 'browse.tilesSmall' : 'browse.tilesLarge') })}</span>
						</button>
						{#if !view}
							{@render filtersButton('flex lg:hidden')}
						{:else}
							{@render filtersButton('hidden sm:flex lg:hidden')}
						{/if}
					{/if}
					{@render view?.()}
				</div>

				<div class="flex min-h-7 flex-wrap items-center gap-1.5">
					{#if backHref && scrollY > 140}
						<a
							href={backHref}
							class="mr-1 flex items-center gap-1.5 rounded-md py-0.5 pr-2 text-sm text-muted transition-colors hover:text-foreground"
							title="Back (Backspace)"
						>
							<span aria-hidden="true">‹</span>
							<span class="max-w-40 truncate font-medium text-foreground">{name || 'Library'}</span>
						</a>
						<span class="mr-1 text-muted/40" aria-hidden="true">·</span>
					{/if}
					<p class={['mr-1 text-sm text-muted', bare ? 'hidden' : ''].join(' ')} aria-live="polite">
						{#if loading && items.length === 0}
							Loading…
						{:else if narrowed}
							<span class="text-foreground">{results.length}</span> of {all.length} {all.length === 1 ? 'title' : 'titles'}
						{:else}
							{all.length} {all.length === 1 ? 'title' : 'titles'}
						{/if}
						{#if loading && items.length > 0}<span class="text-muted/70"> · loading the rest…</span>{/if}
					</p>
					{#each chips as chip (chip.label)}
						<button
							type="button"
							class="flex items-center gap-1 rounded-full border border-accent/40 bg-accent/10 py-0.5 pl-2.5 pr-1.5 text-xs text-foreground transition-colors hover:border-accent/70"
							aria-label={`Remove filter ${chip.label}`}
							onclick={chip.off}
						>
							{chip.label} <X class="size-3 opacity-70" />
						</button>
					{/each}
					{#if narrowed}
						<button type="button" class="rounded-full px-2 py-0.5 text-xs text-muted hover:text-foreground" onclick={clearAll}>
							Clear <kbd class="ml-0.5 hidden opacity-70 md:inline-flex">C</kbd>
						</button>
					{/if}
					<!-- A phone's first row holds the search and the view switch; sort and filters come here. -->
					{#if view && !bare}
						{@render filtersButton('ms-auto flex sm:hidden')}
					{/if}
				</div>
			</div>
		{/if}

		<div bind:this={gridTop}></div>
		{#if error && items.length === 0}
			<ErrorState message={error} retry={() => void load()} />
		{:else if (loading && items.length === 0) || (!held && results.length > 0)}
			<div class={framedGrid('portrait', dense)}>
				{#each Array(8) as _}
					<Skeleton class="aspect-[2/3] w-full rounded-xl" />
				{/each}
			</div>
		{:else if all.length === 0}
			{#if empty}
				{@render empty()}
			{:else}
				<EmptyState title="Nothing indexed here" description="Once a scan finds files for this library they will appear here.">
					{#snippet icon()}<Film class="size-6 text-muted" />{/snippet}
				</EmptyState>
			{/if}
		{:else if results.length === 0}
			<div class="py-14 text-center">
				<p class="text-sm text-foreground">
					{filters.q.trim() ? `Nothing matches “${filters.q.trim()}”${active > 0 ? ' with these filters' : ''}.` : 'No title passes these filters.'}
				</p>
				<div class="mt-4 flex justify-center">
					<Button variant="secondary" onclick={clearAll}>Clear search and filters</Button>
				</div>
			</div>
		{:else}
			<!-- Arrows move between titles; up from the first row returns to the search.
			     A card scrolled to stops below the header and this sticky bar. -->
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div bind:this={gridEl} class={[framedGrid(frame, dense), '[&>*]:scroll-mt-48 [&>*]:scroll-mb-24 md:[&>*]:scroll-mt-52'].join(' ')} onkeydown={(e) => gridKeys(e, () => searchBox)}>
				{#each visible as group, i (group.key)}
					{#if group.count > 1}
						<ShowCard
							{group}
							{frame}
							progressOf={(id) => progressMap.get(id)}
							priority={i < 6}
						/>
					{:else}
						<MediaCard
							item={group.items[0]}
							enrichment={enrichmentCache.get(group.items[0].id) ?? null}
							progress={progressMap.get(group.items[0].id) ?? null}
							priority={i < 6}
							{frame}
						/>
					{/if}
				{/each}
			</div>
			{#if visible.length < results.length}
				<div class="h-10" bind:this={sentinel}></div>
			{:else if results.length > 24}
				<p class="pt-8 text-center text-xs text-muted">
					{results.length} titles ·
					<button type="button" class="underline-offset-4 hover:text-foreground hover:underline" onclick={toTop}>back to top</button>
				</p>
			{/if}
			{#if error}
				<p class="mt-4 text-center text-sm text-danger" role="alert">{error}</p>
			{/if}
		{/if}
	</div>
</div>

{#if scrollY > 700}
	<button
		type="button"
		class="fixed bottom-24 right-5 z-30 flex h-10 items-center gap-2 rounded-full border border-line bg-surface/95 pl-3 pr-3.5 text-sm text-muted shadow-[0_10px_30px_rgba(0,0,0,0.5)] backdrop-blur transition-colors hover:border-white/20 hover:text-foreground md:bottom-8 md:right-8"
		aria-label="Back to top"
		onclick={toTop}
	>
		<ArrowUp class="size-4" />
		<span class="hidden sm:inline">Top</span>
		<kbd class="hidden opacity-60 md:inline-flex">T</kbd>
	</button>
{/if}

<Modal bind:open={sheet} title="Filters">
	<div class="mt-4">
		{@render panel('genre-find-sheet', true)}
	</div>
	<div class="mt-5 flex items-center justify-between gap-3">
		<Button variant="ghost" disabled={active === 0} onclick={clearAll}>Clear</Button>
		<Button onclick={() => (sheet = false)}>Show {results.length} {results.length === 1 ? 'title' : 'titles'}</Button>
	</div>
</Modal>
