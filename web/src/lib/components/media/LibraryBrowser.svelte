<script lang="ts">
	/**
	 * A library as something to search and narrow, not a wall to scroll.
	 *
	 * The whole library is loaded once (titles are small) so that search,
	 * sorting and every filter answer at once and the counts next to each
	 * option are exact. What is set lives in the URL: a view can be
	 * bookmarked, and Back returns to it. The grid lists titles, one card
	 * per show (it opens the show's page) or per single file.
	 */
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import Film from '@lucide/svelte/icons/film';
	import Search from '@lucide/svelte/icons/search';
	import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
	import X from '@lucide/svelte/icons/x';
	import type { CatalogItem, Library, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { FIT, FIT_DENSE } from '$lib/components/media/MediaGrid.svelte';
	import MediaCard from '$lib/components/media/MediaCard.svelte';
	import ShowCard from '$lib/components/media/ShowCard.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import {
		ADDED, NO_FILTERS, SHAPES, SORTS, WATCH,
		activeCount, apply, countBy, filtersFromParams, filtersToParams, genreFacets, watchState,
		type Context, type Filters, type Sort
	} from '$lib/utilities/browse';
	import { errorMessage } from '$lib/utilities/errors';
	import { groupItems } from '$lib/utilities/grouping';

	const FETCH = 500;
	const ENRICH = 150;
	const RENDER = 96;
	const GENRES_SHOWN = 24;
	const PREFS = 'lain.library.prefs';

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
			localStorage.setItem(PREFS, JSON.stringify({ sort: defaultSort, dense }));
		} catch {
			/* private window: lasts for this visit */
		}
	}

	async function load(): Promise<void> {
		const mine = ++run;
		items = [];
		total = 0;
		error = null;
		loading = true;
		try {
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
			loading = false;
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
		try {
			const saved = JSON.parse(localStorage.getItem(PREFS) ?? '{}');
			if (SORTS.some((s) => s.value === saved.sort)) defaultSort = saved.sort;
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

	const filters = $derived(filtersFromParams(page.url.searchParams, defaultSort));
	const active = $derived(activeCount(filters));

	function set(patch: Partial<Filters>): void {
		const query = filtersToParams({ ...filters, ...patch }, defaultSort).toString();
		shown = RENDER;
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
	const addedCounts = $derived(
		new Map(ADDED.map((a) => [a.value, all.filter((g) => apply([g], { ...filters, added: a.value }, ctx).length > 0).length]))
	);

	const chips = $derived([
		...filters.genres.map((g) => ({ label: g, off: () => genre(g, 'want') })),
		...filters.exclude.map((g) => ({ label: `not ${g}`, off: () => genre(g, 'not') })),
		...(filters.watch ? [{ label: WATCH.find((w) => w.value === filters.watch)!.label, off: () => set({ watch: '' }) }] : []),
		...(filters.shape ? [{ label: SHAPES.find((s) => s.value === filters.shape)!.label, off: () => set({ shape: '' }) }] : []),
		...(filters.kind ? [{ label: filters.kind, off: () => set({ kind: '' }) }] : []),
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
		}
	}
</script>

<svelte:window onkeydown={onKey} />

{#snippet option(label: string, count: number | undefined, on: boolean, pick: () => void)}
	<button
		type="button"
		aria-pressed={on}
		disabled={!on && !count}
		class={[
			'flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-sm transition-colors disabled:opacity-35',
			on ? 'bg-accent/15 text-foreground' : 'text-muted hover:bg-surface-hover hover:text-foreground'
		].join(' ')}
		onclick={pick}
	>
		<span class="truncate">{label}</span>
		<span class="font-mono text-[11px] tabular-nums opacity-70">{count ?? 0}</span>
	</button>
{/snippet}

{#snippet heading(label: string)}
	<h3 class="mb-1 px-2 font-mono text-[10px] uppercase tracking-[0.16em] text-muted/80">{label}</h3>
{/snippet}

{#snippet panel(findId: string)}
	<div class="space-y-5">
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
		<div>
			{@render heading('Progress')}
			{#each WATCH as w (w.value)}
				{@render option(w.label, watchCounts.get(w.value), filters.watch === w.value, () => toggle('watch', w.value))}
			{/each}
		</div>
		{#if (shapeCounts.get('series') ?? 0) > 0 && (shapeCounts.get('single') ?? 0) > 0}
			<div>
				{@render heading('Shape')}
				{#each SHAPES as s (s.value)}
					{@render option(s.label, shapeCounts.get(s.value), filters.shape === s.value, () => toggle('shape', s.value))}
				{/each}
			</div>
		{/if}
		{#if kinds.length > 1}
			<div>
				{@render heading('Kind')}
				{#each kinds as k (k)}
					{@render option(k, kindCounts.get(k), filters.kind === k, () => toggle('kind', k))}
				{/each}
			</div>
		{/if}
		<div>
			{@render heading('Added')}
			{#each ADDED as a (a.value)}
				{@render option(a.label, addedCounts.get(a.value), filters.added === a.value, () => toggle('added', a.value))}
			{/each}
		</div>
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
									'flex min-w-0 flex-1 items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-sm transition-colors',
									want ? 'bg-accent/15 text-foreground' : not ? 'text-muted/60 line-through' : 'text-muted hover:bg-surface-hover hover:text-foreground'
								].join(' ')}
								onclick={() => genre(g.value, 'want')}
							>
								<span class="truncate">{g.value}</span>
								<span class="font-mono text-[11px] tabular-nums opacity-70">{g.count}</span>
							</button>
							<button
								type="button"
								aria-pressed={not}
								aria-label={not ? `Stop hiding ${g.value}` : `Hide ${g.value}`}
								title={not ? 'Stop hiding' : 'Hide titles with this genre'}
								class={[
									'flex size-7 shrink-0 items-center justify-center rounded-md text-sm transition-opacity hover:bg-surface-hover hover:text-foreground',
									not ? 'text-danger opacity-100' : 'text-muted opacity-0 focus-visible:opacity-100 group-hover/row:opacity-100'
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

<div class="lg:grid lg:grid-cols-[14.5rem_minmax(0,1fr)] lg:gap-8">
	<aside class="hidden lg:block" aria-label="Filters">
		<div class="sticky top-24 -ml-2 max-h-[calc(100vh-7rem)] overflow-y-auto pb-6 pr-1">
			{@render panel('genre-find')}
		</div>
	</aside>

	<div class="min-w-0 space-y-4">
		<div class="z-20 -mx-1 space-y-3 bg-background/92 px-1 py-2 backdrop-blur md:sticky md:top-[5.25rem]">
			<div class="flex items-center gap-2">
				<div class="relative min-w-0 flex-1">
					<Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted" aria-hidden="true" />
					<input
						bind:this={searchBox}
						type="search"
						value={text}
						oninput={(e) => onSearch(e.currentTarget.value)}
						placeholder="Search this library"
						aria-label="Search this library"
						autocomplete="off"
						spellcheck="false"
						enterkeyhint="search"
						class="h-10 w-full rounded-md border border-line bg-field pl-9 pr-12 text-sm text-foreground placeholder:text-muted/70 transition-colors hover:border-white/15 focus:border-accent/60 focus:outline-none [&::-webkit-search-cancel-button]:hidden"
					/>
					{#if text}
						<button type="button" aria-label="Clear search" class="absolute right-1.5 top-1/2 flex size-7 -translate-y-1/2 items-center justify-center rounded-md text-muted hover:bg-surface-hover hover:text-foreground" onclick={() => { onSearch(''); searchBox?.focus(); }}>
							<X class="size-4" />
						</button>
					{:else}
						<kbd class="pointer-events-none absolute right-3 top-1/2 hidden -translate-y-1/2 font-mono text-[11px] text-muted/70 md:block">/</kbd>
					{/if}
				</div>
				<div class="w-36 shrink-0 sm:w-44">
					<Select
						aria-label="Sort"
						bind:value={sortValue}
						options={SORTS.map((s) => ({ value: s.value, label: s.label }))}
						onValueChange={() => {
							const next = sortValue as Sort;
							// Ask for it while it is not yet the default, so the
							// URL keeps it, then remember it for next time.
							set({ sort: next });
							defaultSort = next;
							savePrefs();
						}}
					/>
				</div>
				<button
					type="button"
					class="hidden h-10 shrink-0 rounded-md border border-line px-3 font-mono text-[11px] uppercase tracking-[0.12em] text-muted transition-colors hover:border-white/15 hover:text-foreground sm:block"
					aria-pressed={dense}
					title="Tile size"
					onclick={() => {
						dense = !dense;
						savePrefs();
					}}
				>
					{dense ? 'Small' : 'Large'}
				</button>
				<button
					type="button"
					class={['flex h-10 shrink-0 items-center gap-2 rounded-md border px-3 text-sm lg:hidden', active > 0 ? 'border-accent/60 text-foreground' : 'border-line text-muted'].join(' ')}
					aria-label={active > 0 ? `Filters (${active} active)` : 'Filters'}
					onclick={() => (sheet = true)}
				>
					<SlidersHorizontal class="size-4" />
					{#if active > 0}<span class="font-mono text-xs tabular-nums">{active}</span>{/if}
				</button>
			</div>

			<div class="flex min-h-7 flex-wrap items-center gap-1.5">
				<p class="mr-1 text-sm text-muted" aria-live="polite">
					{#if loading && items.length === 0}
						Loading…
					{:else if narrowed}
						<span class="text-foreground">{results.length}</span> of {all.length} {all.length === 1 ? 'title' : 'titles'}
					{:else}
						{all.length} {all.length === 1 ? 'title' : 'titles'}{#if items.length !== all.length}&nbsp;· {total} files{/if}
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
						Clear <kbd class="ml-0.5 hidden font-mono text-[10px] opacity-70 md:inline">C</kbd>
					</button>
				{/if}
			</div>
		</div>

		{#if error && items.length === 0}
			<ErrorState message={error} retry={() => void load()} />
		{:else if loading && items.length === 0}
			<div class={dense ? FIT_DENSE : FIT}>
				{#each Array(8) as _}
					<Skeleton class="aspect-video h-[var(--card-h)]" />
				{/each}
			</div>
		{:else if all.length === 0}
			<EmptyState title="Nothing indexed here" description="Once a scan finds files for this library they will appear here.">
				{#snippet icon()}<Film class="size-6 text-muted" />{/snippet}
			</EmptyState>
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
			<div class={dense ? FIT_DENSE : FIT}>
				{#each visible as group, i (group.key)}
					{#if group.count > 1}
						<ShowCard
							{group}
							fit
							progress={group.items.map((it) => progressMap.get(it.id)).find(Boolean) ?? null}
							priority={i < 6}
						/>
					{:else}
						<MediaCard
							item={group.items[0]}
							enrichment={enrichmentCache.get(group.items[0].id) ?? null}
							progress={progressMap.get(group.items[0].id) ?? null}
							priority={i < 6}
							fit
						/>
					{/if}
				{/each}
			</div>
			{#if visible.length < results.length}
				<div class="h-10" bind:this={sentinel}></div>
			{/if}
			{#if error}
				<p class="mt-4 text-center text-sm text-danger" role="alert">{error}</p>
			{/if}
		{/if}
	</div>
</div>

<Modal bind:open={sheet} title="Filters">
	<div class="mt-4">
		{@render panel('genre-find-sheet')}
	</div>
	<div class="mt-5 flex items-center justify-between gap-3">
		<Button variant="ghost" disabled={active === 0} onclick={clearAll}>Clear</Button>
		<Button onclick={() => (sheet = false)}>Show {results.length} {results.length === 1 ? 'title' : 'titles'}</Button>
	</div>
</Modal>
