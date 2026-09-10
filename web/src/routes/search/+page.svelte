<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import Search from '@lucide/svelte/icons/search';
	import SearchX from '@lucide/svelte/icons/search-x';
	import type { CatalogItem } from '$lib/api/types';
	import { api } from '$lib/api';
	import MediaGrid from '$lib/components/media/MediaGrid.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { debounce } from '$lib/utilities/debounce';
	import { errorMessage } from '$lib/utilities/errors';

	const PAGE_SIZE = 60;

	let query = $state(page.url.searchParams.get('q') ?? '');
	let input = $state<HTMLInputElement | null>(null);
	let items = $state<CatalogItem[]>([]);
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
		void run(query, true);
	}

	function clear(): void {
		query = '';
		syncUrl('');
		debounced.cancel();
		void run('', true);
		input?.focus();
	}

	onMount(() => {
		if (query.trim() !== '') void run(query, true);
	});

	const emptyResults = $derived(!loading && searchedFor !== '' && items.length === 0 && !error);
	const showIdle = $derived(searchedFor === '' && !loading);
</script>

<svelte:head><title>{query ? `${query} — Search — Lain` : 'Search — Lain'}</title></svelte:head>

<div class="space-y-6">
	<header class="space-y-3">
		<h1 class="text-xl font-semibold tracking-tight text-foreground">Search</h1>
		<form class="relative max-w-2xl" onsubmit={submit} role="search">
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
				class="h-12 w-full rounded-card border border-line bg-surface pl-10 pr-4 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none"
			/>
		</form>
		{#if searchedFor !== '' && !loading}
			<p class="text-sm text-muted" aria-live="polite">
				{total} {total === 1 ? 'result' : 'results'} for “{searchedFor}”
			</p>
		{/if}
	</header>

	{#if error}
		<ErrorState message={error} retry={() => void run(query, true)} />
	{:else if showIdle}
		<EmptyState
			title="Search your library"
			description="Type a title. Matching is case-insensitive and covers what the catalog knows, not raw filenames."
		>
			{#snippet icon()}<Search class="size-6 text-muted" />{/snippet}
		</EmptyState>
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
		<MediaGrid {items} />
		{#if items.length < total}
			<div class="mt-8 flex justify-center">
				<Button variant="secondary" loading={loading} onclick={() => void run(searchedFor, false)}>
					<ArrowDown class="size-4" /> Load more
				</Button>
			</div>
		{/if}
	{/if}
</div>
