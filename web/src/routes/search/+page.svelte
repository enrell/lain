<script lang="ts">
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import { ensureLibraries } from '$lib/stores/media-cache.svelte';
	import X from '@lucide/svelte/icons/x';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import Search from '@lucide/svelte/icons/search';
	import SearchX from '@lucide/svelte/icons/search-x';
	import type { CatalogItem } from '$lib/api/types';
	import { api } from '$lib/api';
	import MediaCard from '$lib/components/media/MediaCard.svelte';
	import { FIT } from '$lib/components/media/MediaGrid.svelte';
	import ShowCard from '$lib/components/media/ShowCard.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { groupItems } from '$lib/utilities/grouping';
	import { debounce } from '$lib/utilities/debounce';
	import { errorMessage } from '$lib/utilities/errors';

	const PAGE_SIZE = 60;

	let query = $state(page.url.searchParams.get('q') ?? '');
	let input = $state<HTMLInputElement | null>(null);
	let items = $state<CatalogItem[]>([]);
	const groups = $derived(groupItems(items));
	let total = $state(0);
	let loading = $state(false);
	let error = $state<string | null>(null);
	let searchedFor = $state('');
	let controller: AbortController | null = null;

	async function run(q: string, reset: boolean): Promise<void> {
		const term = q.trim();
		controller?.abort();
		const abort = new AbortController();
		controller = abort;

		if (term === '') {
			items = [];
			total = 0;
			searchedFor = '';
			error = null;
			loading = false;
			return;
		}

		const offset = reset ? 0 : items.length;
		loading = true;
		error = null;
		try {
			const result = await api.search.query({
				q: term,
				limit: PAGE_SIZE,
				offset,
				signal: abort.signal
			});
			items = reset ? result.items : [...items, ...result.items];
			total = result.total;
			searchedFor = term;
			await ensureEnrichments(result.items.map((i) => i.id));
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
		const target = q.trim() === '' ? '/search' : `/search?q=${encodeURIComponent(q.trim())}`;
		if (`${page.url.pathname}${page.url.search}` !== target) {
			void goto(target, { replaceState: true, keepFocus: true, noScroll: true });
		}
	}

	function onInput(event: Event): void {
		query = (event.currentTarget as HTMLInputElement).value;
		syncUrl(query);
		debounced(query);
	}

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		debounced.cancel();
		remember(query);
		void run(query, true);
	}

	// The last searches that were meant (sent with Enter, or followed into a
	// result), kept in this browser: typing passes through too many half words.
	const RECENT_KEY = 'lain.search.recent';
	let recent = $state<string[]>([]);
	function remember(q: string): void {
		const term = q.trim();
		if (term.length < 2) return;
		recent = [term, ...recent.filter((r) => r.toLowerCase() !== term.toLowerCase())].slice(0, 6);
		try {
			localStorage.setItem(RECENT_KEY, JSON.stringify(recent));
		} catch {
			// A browser that keeps nothing still searches.
		}
	}
	function forget(): void {
		recent = [];
		try {
			localStorage.removeItem(RECENT_KEY);
		} catch {
			// Nothing was kept.
		}
	}
	function again(term: string): void {
		query = term;
		syncUrl(term);
		debounced.cancel();
		remember(term);
		void run(term, true);
	}

	function clear(): void {
		query = '';
		syncUrl('');
		debounced.cancel();
		void run('', true);
		input?.focus();
	}

	onMount(() => {
		try {
			const kept: unknown = JSON.parse(localStorage.getItem(RECENT_KEY) ?? '[]');
			if (Array.isArray(kept)) recent = kept.filter((r): r is string => typeof r === 'string').slice(0, 6);
		} catch {
			// Unreadable: start with none.
		}
		if (query.trim() !== '') void run(query, true);
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

<svelte:head><title>{query ? `${query} — Search — Lain` : 'Search — Lain'}</title></svelte:head>

<div class="space-y-6">
	<header class="space-y-3">
		<h1 class="text-xl font-semibold tracking-tight text-foreground">Search</h1>
		<form class="relative" onsubmit={submit} role="search">
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
				}}
				type="search"
				placeholder="Titles, not filenames"
				aria-label="Search your library"
				autocapitalize="none"
				spellcheck={false}
				class="h-12 w-full rounded-card border border-line bg-surface pl-10 pr-12 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none [&::-webkit-search-cancel-button]:hidden"
			/>
			{#if query !== ''}
				<button
					type="button"
					class="absolute inset-y-0 right-0 flex w-12 items-center justify-center text-muted hover:text-foreground"
					aria-label="Clear the search"
					onclick={() => { clear(); input?.focus(); }}
				>
					<X class="size-4" aria-hidden="true" />
				</button>
			{/if}
		</form>
		{#if searchedFor !== '' && !loading}
			<p class="text-sm text-muted" aria-live="polite">
				{groups.length} {groups.length === 1 ? 'title' : 'titles'} for “{searchedFor}”
			</p>
		{/if}
	</header>

	{#if error}
		<ErrorState message={error} retry={() => void run(query, true)} />
	{:else if showIdle}
		<!-- Before a query: what was searched lately, and the libraries to start from. -->
		{#if recent.length > 0}
			<section aria-label="Recent searches">
				<div class="flex items-center justify-between gap-3">
					<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-foreground/65">Recent</h2>
					<button type="button" class="rounded-md px-2 py-1 text-xs text-muted hover:text-foreground" onclick={forget}>Clear</button>
				</div>
				<div class="mt-2 flex flex-wrap gap-2">
					{#each recent as term (term)}
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
		<section aria-label="Browse">
			<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-foreground/65">Browse</h2>
			<ul class="mt-2 divide-y divide-line/50">
				{#each browse as lib (lib.id)}
					<li>
						<a href={`/library/${lib.id}`} class="flex min-h-12 items-center justify-between gap-3 rounded-md px-1 text-sm font-medium text-foreground hover:bg-surface-hover">
							{lib.name}
							<ChevronRight class="size-4 text-muted" aria-hidden="true" />
						</a>
					</li>
				{/each}
			</ul>
			<p class="mt-4 text-sm text-muted">Search matches titles, whatever their case; it does not look at file names.</p>
		</section>
	{:else if loading && items.length === 0}
		<div class="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
			{#each Array(12) as _}
				<Skeleton class="h-56" />
			{/each}
		</div>
	{:else if emptyResults}
		<EmptyState
			title={`No matches for “${searchedFor}”`}
			description="Try a shorter title, or check whether a scan has indexed the library."
		>
			{#snippet icon()}<SearchX class="size-6 text-muted" />{/snippet}
		</EmptyState>
	{:else}
		<!-- A search answers with titles, not files: twelve episodes of one
		     show are one card that opens the show. -->
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class={FIT} onclick={() => remember(searchedFor)}>
			{#each groups as group, i (group.key)}
				{#if group.count > 1}
					<ShowCard {group} fit priority={i < 6} />
				{:else}
					<MediaCard item={group.items[0]} enrichment={enrichmentCache.get(group.items[0].id) ?? null} priority={i < 6} fit />
				{/if}
			{/each}
		</div>
		{#if items.length < total}
			<div class="mt-8 flex justify-center">
				<Button variant="secondary" loading={loading} onclick={() => void run(searchedFor, false)}>
					<ArrowDown class="size-4" /> Load more
				</Button>
			</div>
		{/if}
	{/if}
	{#if !error && !showIdle && !loading && groups.length <= 6 && browse.length > 0}
		<!-- A short answer leaves the page with somewhere to go on to. -->
		<nav aria-label="Browse" class="border-t border-line/50 pt-5">
			<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-foreground/65">{emptyResults ? 'Browse instead' : 'Or browse'}</h2>
			<div class="mt-2 flex flex-wrap gap-2">
				{#each browse as lib (lib.id)}
					<a href={`/library/${lib.id}`} class="inline-flex min-h-9 items-center gap-1.5 rounded-full border border-line px-3.5 text-sm text-foreground hover:bg-surface-hover">
						{lib.name} <ChevronRight class="size-3.5 text-muted" aria-hidden="true" />
					</a>
				{/each}
			</div>
		</nav>
	{/if}
</div>
