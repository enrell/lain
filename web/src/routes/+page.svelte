<script lang="ts">
	import { isReadable } from '$lib/reader/kinds';
	import { onMount } from 'svelte';
	import Clapperboard from '@lucide/svelte/icons/clapperboard';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import ScanLine from '@lucide/svelte/icons/scan-line';
	import type { CatalogItem, Library, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Hero from '$lib/components/media/Hero.svelte';
	import MediaRow from '$lib/components/media/MediaRow.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { ensureEnrichments, ensureItems, ensureLibraries } from '$lib/stores/media-cache.svelte';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import { scan } from '$lib/stores/scan.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { isInProgress, nextUpAfter, sortByRecent } from '$lib/utilities/progress';

	let loading = $state(true);
	let error = $state<string | null>(null);
	let libraries = $state<Library[]>([]);
	let continueItems = $state<CatalogItem[]>([]);
	// Reading and watching are different habits: two rows, not one mixed shelf.
	// What the hero resumes (the first of them) is not repeated as the first card of its rail.
	const continueWatching = $derived(continueItems.slice(1).filter((i) => !isReadable(i.kind)));
	const continueReading = $derived(continueItems.slice(1).filter((i) => isReadable(i.kind)));
	let progressMap = $state<Map<string, Progress>>(new Map());
	let nextUpItems = $state<CatalogItem[]>([]);
	// The titles kept as favourites (the social module's list), newest first.
	let favoriteItems = $state<CatalogItem[]>([]);
	const nextUpWatching = $derived(nextUpItems.filter((i) => !isReadable(i.kind)));
	const nextUpReading = $derived(nextUpItems.filter((i) => isReadable(i.kind)));
	let recent = $state<CatalogItem[]>([]);
	let libraryRows = $state<{ lib: Library; items: CatalogItem[] }[]>([]);
	let catalogTotal = $state(0);

	const NEXT_UP_TITLES = 6;

	// One suggestion per show: the episode after the most recently
	// finished one. A failed lookup only drops that show's suggestion.
	async function loadNextUp(list: Progress[]): Promise<CatalogItem[]> {
		const all = new Map(list.map((p) => [p.item_id, p]));
		const finished = sortByRecent(list.filter((p) => p.completed && p.item_id !== ''));
		const seen = new Set<string>();
		const found: CatalogItem[] = [];
		for (const p of finished) {
			if (found.length >= NEXT_UP_TITLES || seen.size >= NEXT_UP_TITLES * 3) break;
			if (seen.has(p.item_id)) continue;
			try {
				const { items } = await api.catalog.episodes(p.item_id);
				for (const e of items) seen.add(e.id);
				const next = nextUpAfter(items, p.item_id, all);
				if (next) found.push(next);
			} catch {
				seen.add(p.item_id);
			}
		}
		return found;
	}

	// A server without the social module, or an account with no favourites, has no such rail.
	async function loadFavorites(): Promise<CatalogItem[]> {
		try {
			const ids = (await api.social.settings()).favorites.map((f) => f.item_id ?? '').filter((id) => id !== '');
			return (await ensureItems(ids.reverse())).filter((item): item is CatalogItem => item !== undefined);
		} catch {
			return [];
		}
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const [progressList, libs, recentPage] = await Promise.all([
				api.me.continueWatching(),
				ensureLibraries(),
				api.catalog.list({ sort: 'recent', limit: 18 })
			]);
			libraries = libs;
			recent = recentPage.items;
			catalogTotal = recentPage.total;

			// A video counts as started after a few seconds; a book counts from its
			// first page (its position is a page number, not seconds), so what is
			// started is decided once the items are known.
			const touched = sortByRecent(
				progressList.filter((p) => p.item_id !== '' && !p.completed && p.position_sec >= 1)
			).slice(0, 36);
			const touchedItems = await ensureItems(touched.map((p) => p.item_id));
			const started = touched
				.map((p, i) => ({ p, item: touchedItems[i] }))
				.filter((row): row is { p: Progress; item: CatalogItem } => row.item !== undefined && (isReadable(row.item.kind) || isInProgress(row.p)))
				.slice(0, 12);
			progressMap = new Map(started.map((row) => [row.p.item_id, row.p]));
			continueItems = started.map((row) => row.item);
			nextUpItems = await loadNextUp(progressList);
			favoriteItems = await loadFavorites();

			// One row per library when there is more than one: the
			// catalog filter runs server-side, nothing is preloaded.
			const rows = await Promise.all(
				libs.slice(0, 4).map(async (lib) => ({
					lib,
					items: (await api.catalog.list({ libraryId: lib.id, limit: 12 })).items
				}))
			);
			libraryRows = libs.length > 1 ? rows.filter((r) => r.items.length > 0) : [];

			const ids = [
				...continueItems.map((i) => i.id),
				...nextUpItems.map((i) => i.id),
				...favoriteItems.map((i) => i.id),
				...recent.map((i) => i.id),
				...libraryRows.flatMap((r) => r.items.map((i) => i.id))
			];
			await ensureEnrichments(ids);
		} catch (err) {
			error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
		void scan.watch();
	});

	async function startScan(): Promise<void> {
		try {
			await scan.start();
		} catch (err) {
			error = errorMessage(err);
		}
	}
</script>

<svelte:head><title>Home — Lain</title></svelte:head>

<div>
	{#if loading}
		<div class="space-y-12 pb-20">
			<Skeleton class="h-[60svh] min-h-[24rem] w-full rounded-none md:h-[74svh]" />
			<div class="mx-auto w-full max-w-[1800px] space-y-5 px-5 sm:px-8 lg:px-10">
				<Skeleton class="h-7 w-52" />
				<div class="flex gap-4 overflow-hidden">
					{#each Array(5) as _}
						<Skeleton class="aspect-video w-72 shrink-0 rounded-xl" />
					{/each}
				</div>
			</div>
		</div>
	{:else if error}
		<div class="mx-auto w-full max-w-[1800px] px-5 sm:px-8 lg:px-10 pb-24 pt-24 md:pt-32">
			<ErrorState message={error} retry={() => void load()} />
		</div>
	{:else if libraries.length === 0}
		<div class="mx-auto w-full max-w-[1800px] px-5 sm:px-8 lg:px-10 pb-24 pt-24 md:pt-32">
			<EmptyState
				title="No media yet"
				description={session.isAdmin
					? 'Point Lain at a directory on this server and run a scan. Files stay where they are.'
					: 'An administrator has not configured a library yet.'}
			>
				{#snippet icon()}<Clapperboard class="size-6 text-muted" />{/snippet}
				{#if session.isAdmin}
					<LinkButton href="/settings/libraries">
						<FolderPlus class="size-4" /> Add a library
					</LinkButton>
				{/if}
			</EmptyState>
		</div>
	{:else if catalogTotal === 0}
		<div class="mx-auto w-full max-w-[1800px] px-5 sm:px-8 lg:px-10 pb-24 pt-24 md:pt-32">
			<EmptyState
				title="Library hasn't been scanned"
				description="Libraries are configured, but no media has been indexed yet."
			>
				{#snippet icon()}<ScanLine class="size-6 text-muted" />{/snippet}
				{#if session.isAdmin}
					<Button onclick={() => void startScan()} loading={scan.running}>
						<ScanLine class="size-4" /> Scan now
					</Button>
				{/if}
			</EmptyState>
		</div>
	{:else}
		{@const hero = continueItems[0] ?? recent[0]}
		{#if hero}
			<Hero
				item={hero}
				enrichment={enrichmentCache.get(hero.id) ?? null}
				resume={progressMap.has(hero.id)}
				progress={progressMap.get(hero.id) ?? null}
			/>
		{/if}
		<section id="home-library" class="home-library relative bg-background pb-24 pt-10 sm:pt-12 md:pb-20 md:pt-10">
			<div class="mx-auto w-full max-w-[1800px] space-y-14 md:space-y-16 px-5 sm:px-8 lg:px-10">
				{#if scan.running}
					<div class="flex"><Badge tone="accent"><ScanLine class="size-3" /> Scanning…</Badge></div>
				{:else if scan.status?.state === 'error'}
					<div class="flex"><Badge tone="danger">Last scan failed</Badge></div>
				{/if}

				{#if continueWatching.length > 0}
					<MediaRow title="Continue watching" href="/library" actionLabel="Browse all" items={continueWatching} {progressMap} layout="landscape" />
				{/if}

				{#if continueReading.length > 0}
					<MediaRow title="Continue reading" href="/library" actionLabel="Browse all" items={continueReading} {progressMap} layout="poster" />
				{/if}

				{#if favoriteItems.length > 0}
					<MediaRow title="Favorites" href={session.user ? `/u/${session.user.id}` : null} actionLabel="All favorites" items={favoriteItems} {progressMap} layout="poster" />
				{/if}

				{#if nextUpWatching.length > 0}
					<MediaRow title="Next up" href="/library" actionLabel="Browse all" items={nextUpWatching} {progressMap} layout="landscape" />
				{/if}

				{#if nextUpReading.length > 0}
					<MediaRow title="Read next" href="/library" actionLabel="Browse all" items={nextUpReading} {progressMap} layout="poster" />
				{/if}

				{#if recent.length > 0}
					<MediaRow title="Recently added" href="/library" items={recent.slice(0, 12)} {progressMap} layout="landscape" />
				{/if}

				{#each libraryRows as row (row.lib.id)}
					<MediaRow title={row.lib.name} href={`/library/${row.lib.id}`} items={row.items} {progressMap} layout={row.lib.type === 'movie' || isReadable(row.lib.type) ? 'poster' : 'landscape'} />
				{/each}
			</div>
		</section>
	{/if}
</div>
