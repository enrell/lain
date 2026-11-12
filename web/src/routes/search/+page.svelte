<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import { ensureLibraries } from '$lib/stores/media-cache.svelte';
	import X from '@lucide/svelte/icons/x';
	import { onMount, tick, untrack } from 'svelte';
	import { afterNavigate, beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import Search from '@lucide/svelte/icons/search';
	import SearchX from '@lucide/svelte/icons/search-x';
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import RelatedRow from '$lib/components/media/RelatedRow.svelte';
	import TitleGrid from '$lib/components/media/TitleGrid.svelte';
	import { framedGrid } from '$lib/components/media/grid';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { t } from '$lib/i18n';
	import { settleFrame, shapesNow } from '$lib/stores/frames';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { pictureShape, SAMPLE, type Shape } from '$lib/stores/shapes.svelte';
	import { pagedTitles } from '$lib/stores/titles';
	import { artItem } from '$lib/utilities/browse';
	import { recentSearches } from '$lib/stores/recent-searches.svelte';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { groupItems, type SeriesGroup } from '$lib/utilities/grouping';
	import { debounce } from '$lib/utilities/debounce';
	import { errorMessage } from '$lib/utilities/errors';
	import { isTypingTarget } from '$lib/utilities/guards';
	import { searchHref } from '$lib/utilities/search';

	// Titles a page (files from a server before D-283).
	const PAGE_SIZE = 60;

	let query = $state(page.url.searchParams.get('q') ?? '');
	let input = $state<HTMLInputElement | null>(null);
	let items = $state<CatalogItem[]>([]);
	const groups = $derived(groupItems(items));
	// The shape of each result's picture, as a library grid has it: guessed
	// when the answer is drawn, checked once against its first pictures and
	// held; posters and wide stills are each their own part (TitleGrid).
	let shapes = $state<Map<string, Shape>>(new Map());
	const shapeOf = (group: SeriesGroup): Shape =>
		shapes.get(group.key) ?? untrack(() => pictureShape(group, (id) => enrichmentCache.get(id)));
	let checking = 0;
	let grid = $state<HTMLElement | null>(null);
	let total = $state(0);
	// A server that pages titles (D-283) says how many titles match, and each
	// title it sends is whole; an older one pages files, which are grouped here.
	let byTitle = $state(false);
	// The account's positions, so a result says watched or under way as the
	// same title does in the library.
	let progressMap = $state<Map<string, Progress>>(new Map());
	let loading = $state(false);
	let error = $state<string | null>(null);
	let searchedFor = $state('');
	let controller: AbortController | null = null;

	// ---- where you were ----------------------------------------------------
	// Back from a result lands where the viewer was, as a library grid does.
	// SvelteKit restores the scroll before the results are fetched, when the
	// page is only as tall as its header, so the spot and how many results
	// were loaded are kept per URL and restored once they are drawn again.
	const PLACE = 'lain.search.place:';
	const placeKey = (url: URL) => PLACE + url.pathname + url.search;
	let restoreTo: { y: number; count: number } | null = null;
	beforeNavigate((nav) => {
		if (!nav.from) return;
		try {
			sessionStorage.setItem(placeKey(nav.from.url), JSON.stringify({ y: window.scrollY, count: loaded() }));
		} catch {
			/* no storage: Back lands at the top */
		}
	});
	afterNavigate((nav) => {
		if (nav.type !== 'popstate') return;
		try {
			const saved = JSON.parse(sessionStorage.getItem(placeKey(page.url)) ?? 'null');
			if (saved && saved.y > 0) restoreTo = { y: Number(saved.y), count: Number(saved.count) || 0 };
		} catch {
			/* stay at the top */
		}
	});
	/** How much of the answer is here, in what `total` counts: titles, or files from an older server. */
	const loaded = () => (byTitle ? groups.length : items.length);

	/** Scrolls back to the saved spot once as many results as there were are drawn. */
	async function restorePlace(term: string): Promise<void> {
		const place = restoreTo;
		if (!place) return;
		if (loaded() < place.count && loaded() < total) {
			// The viewer had loaded more: the same pages come back first.
			await run(term, false);
			return;
		}
		restoreTo = null;
		await tick();
		window.scrollTo({ top: place.y });
	}

	async function run(q: string, reset: boolean): Promise<void> {
		const term = q.trim();
		controller?.abort();
		const abort = new AbortController();
		controller = abort;

		if (term === '') {
			restoreTo = null;
			checking += 1;
			items = [];
			total = 0;
			searchedFor = '';
			error = null;
			loading = false;
			return;
		}

		const offset = reset ? 0 : loaded();
		loading = true;
		error = null;
		try {
			const result = await api.search.query({
				q: term,
				limit: PAGE_SIZE,
				offset,
				group: 'title',
				signal: abort.signal
			});
			const next = reset ? result.items : [...items, ...result.items];
			// The overlays carry the titles' names and say which pictures are
			// posters: one request, no picture (the first of each title is
			// enough for its card; a long show's every episode would not fit).
			await ensureEnrichments(groupItems(result.items).map((g) => artItem(g).id));
			if (abort.signal.aborted) return;
			// Drawn at once, in the shapes guessed from what is known before a
			// picture loads: waiting for the pictures left the page blank for
			// seconds on a cold cache. More of the same answer keeps them.
			const shown = groupItems(next);
			shapes = reset ? shapesNow(shown) : new Map([...shapesNow(shown), ...untrack(() => shapes)]);
			items = next;
			total = result.total;
			byTitle = pagedTitles(result);
			searchedFor = term;
			void restorePlace(term);
			if (reset) {
				// Then checked once against its first pictures: a shape changes
				// only if the guess was wrong, and is held after that.
				const mine = ++checking;
				void settleFrame(shown).then(() => {
					if (mine === checking) shapes = new Map([...untrack(() => shapes), ...shapesNow(shown.slice(0, SAMPLE))]);
				});
			}
		} catch (err) {
			if (err instanceof DOMException && err.name === 'AbortError') return;
			error = errorMessage(err);
		} finally {
			if (controller === abort) loading = false;
		}
	}

	const debounced = debounce((q: string) => {
		void run(q, true);
	}, 300);

	function syncUrl(q: string): void {
		const target = searchHref(q);
		if (`${page.url.pathname}${page.url.search}` !== target) {
			void goto(target, { replaceState: true, keepFocus: true, noScroll: true });
		}
	}

	function onInput(event: Event): void {
		restoreTo = null;
		query = (event.currentTarget as HTMLInputElement).value;
		syncUrl(query);
		debounced(query);
	}

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		debounced.cancel();
		recentSearches.remember(query);
		void run(query, true);
	}

	function again(term: string): void {
		query = term;
		syncUrl(term);
		debounced.cancel();
		recentSearches.remember(term);
		void run(term, true);
	}

	/*
	 * From md up the header's field is this page's field: it types into
	 * the URL (already debounced) and the results follow at once. This
	 * page's own typing writes the URL too, and finds it unchanged.
	 */
	$effect(() => {
		const q = page.url.searchParams.get('q') ?? '';
		if (q === untrack(() => query.trim())) return;
		restoreTo = null;
		query = q;
		debounced.cancel();
		void run(q, true);
	});

	// The field shown on this screen: the header's from md up, this page's on a phone.
	function field(): HTMLInputElement | null {
		const header = document.getElementById('header-search');
		if (header instanceof HTMLInputElement && header.getClientRects().length > 0) return header;
		return input && input.getClientRects().length > 0 ? input : null;
	}

	usePageKeys(() =>
		items.length > 0
			? [
					{ keys: ['↓'], label: t('pageKeys.resultsMove') },
					{ keys: ['↑', '↓', '←', '→'], label: t('pageKeys.gridMove') }
				]
			: []
	);

	// `/` is this page's field where it is the one shown; the header's handler covers the rest.
	function onKey(e: KeyboardEvent): void {
		if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		if (!input || input.getClientRects().length === 0) return;
		e.preventDefault();
		input.focus();
		input.select();
	}

	function clear(): void {
		query = '';
		syncUrl('');
		debounced.cancel();
		void run('', true);
		input?.focus();
	}

	onMount(() => {
		void api.me
			.continueWatching()
			.then((list) => (progressMap = new Map(list.map((p) => [p.item_id, p]))))
			.catch(() => {});
		if (query.trim() !== '') void run(query, true);
		// Arriving with nothing to look for: the cursor is in the search bar.
		else field()?.focus();
	});

	const emptyResults = $derived(!loading && searchedFor !== '' && items.length === 0 && !error);
	const showIdle = $derived(searchedFor === '' && !loading);
	// The libraries, to browse from before anything is typed.
	let browse = $state<{ id: string; name: string }[]>([]);
	$effect(() => {
		// A private library still locked is not offered: it would open onto nothing.
		void ensureLibraries().then((libs) => (browse = libs.filter((l) => !l.locked).map((l) => ({ id: l.id, name: l.name }))));
	});
</script>

<!-- The libraries as chips, before a query and after a short answer alike. -->
{#snippet browseChips()}
	<div class="mt-2 flex flex-wrap gap-2">
		{#each browse as lib (lib.id)}
			<a href={`/library/${lib.id}`} class="inline-flex min-h-9 items-center gap-1.5 rounded-full border border-line px-3.5 text-sm text-foreground hover:bg-surface-hover">
				{lib.name} <ChevronRight class="size-3.5 text-muted" aria-hidden="true" />
			</a>
		{/each}
	</div>
{/snippet}

<svelte:head><title>{query ? t('search.pageTitleQuery', { query }) : t('search.pageTitle')}</title></svelte:head>
<svelte:window onkeydown={onKey} />

<div class="space-y-6">
	<header class="space-y-3">
		<h1 class="page-title">{t('search.heading')}</h1>
		<!-- A phone's field: from md up the header holds the search bar (HeaderSearch). -->
		<form class="relative md:hidden" onsubmit={submit} role="search">
			<Search
				class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted"
				aria-hidden="true"
			/>
			<input
				bind:this={input}
				value={query}
				oninput={onInput}
				onkeydown={(e) => {
					if (e.key === 'Escape' && query !== '') clear();
					else if (e.key === 'ArrowDown' && !e.altKey && !e.ctrlKey && !e.metaKey) {
						// The results are the page: the arrow walks into them.
						const first = grid?.querySelector<HTMLElement>('a[href]');
						if (!first) return;
						e.preventDefault();
						first.focus();
					}
				}}
				type="search"
				placeholder={t('search.placeholder')}
				aria-label={t('search.label')}
				autocapitalize="none"
				spellcheck={false}
				class="h-12 w-full rounded-card border border-line bg-surface pl-10 pr-12 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none [&::-webkit-search-cancel-button]:hidden"
			/>
			{#if query !== ''}
				<button
					type="button"
					class="absolute inset-y-0 right-0 flex w-12 items-center justify-center text-muted hover:text-foreground"
					aria-label={t('search.clear')}
					onclick={() => { clear(); input?.focus(); }}
				>
					<X class="size-4" aria-hidden="true" />
				</button>
			{/if}
		</form>
		{#if searchedFor !== '' && !loading}
			<p class="text-sm text-muted" aria-live="polite">
				{t('search.count', { count: byTitle ? total : groups.length, query: searchedFor })}
			</p>
		{/if}
	</header>

	{#if error}
		<ErrorState message={error} retry={() => void run(query, true)} />
	{:else if showIdle}
		<!-- Before a query: what was searched lately, and the libraries to start from. -->
		{#if recentSearches.items.length > 0}
			<section aria-label={t('search.recentLabel')}>
				<div class="flex items-center justify-between gap-3">
					<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-foreground/65">{t('search.recent')}</h2>
					<button type="button" class="rounded-md px-2 py-1 text-xs text-muted hover:text-foreground" onclick={() => recentSearches.forget()}>{t('search.forget')}</button>
				</div>
				<div class="mt-2 flex flex-wrap gap-2">
					{#each recentSearches.items as term (term)}
						<button
							type="button"
							class="inline-flex min-h-9 items-center gap-2 rounded-full border border-line px-3.5 text-sm text-foreground hover:bg-surface-hover"
							onclick={() => again(term)}
						>
							<Search class="size-3.5 text-muted" aria-hidden="true" />
							{term}
						</button>
					{/each}
				</div>
			</section>
		{/if}
		<section aria-label={t('search.browse')}>
			<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-foreground/65">{t('search.browse')}</h2>
			{@render browseChips()}
			<p class="mt-4 text-sm text-muted">{t('search.matches')}</p>
		</section>
		<!-- Something to look at before a word is typed, sized as Home's rails. -->
		<RelatedRow recent heading={t('search.newest')} />
	{:else if loading && items.length === 0}
		<div class={framedGrid('portrait')}>
			{#each Array(12) as _}
				<Skeleton class="aspect-[2/3] w-full rounded-xl" />
			{/each}
		</div>
	{:else if emptyResults}
		<EmptyState
			title={t('search.emptyTitle', { query: searchedFor })}
			description={t('search.emptyBody')}
		>
			{#snippet icon()}<SearchX class="size-6 text-muted" />{/snippet}
		</EmptyState>
	{:else}
		<!-- A search answers with titles, not files: twelve episodes of one
		     show are one card that opens the show. The library's frames and
		     sizes, so a result looks as the same title does everywhere else.
		     Arrows move between results; up from the first row returns to the
		     field (the header's, or this page's on a phone). -->
		<TitleGrid
			id="search-results"
			bind:element={grid}
			{groups}
			{shapeOf}
			progressOf={(id) => progressMap.get(id)}
			above={field}
			cardClass="[&>*]:scroll-mt-28 [&>*]:scroll-mb-24"
			onclick={() => recentSearches.remember(searchedFor)}
		/>
		{#if loaded() < total}
			<div class="mt-8 flex justify-center">
				<Button variant="secondary" loading={loading} onclick={() => void run(searchedFor, false)}>
					<ArrowDown class="size-4" /> {t('search.loadMore')}
				</Button>
			</div>
		{/if}
	{/if}
	{#if !error && !showIdle && !loading && (byTitle ? total : groups.length) <= 6 && browse.length > 0}
		<!-- A short answer leaves the page with somewhere to go on to. -->
		<nav aria-label={t('search.browse')} class="border-t border-line/50 pt-5">
			<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-foreground/65">{t(emptyResults ? 'search.browseInstead' : 'search.orBrowse')}</h2>
			{@render browseChips()}
		</nav>
	{/if}
</div>
