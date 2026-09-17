<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
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
	import { isInProgress, sortByRecent } from '$lib/utilities/progress';

	let loading = $state(true);
	let error = $state<string | null>(null);
	let libraries = $state<Library[]>([]);
	let continueItems = $state<CatalogItem[]>([]);
	let progressMap = $state<Map<string, Progress>>(new Map());
	let recent = $state<CatalogItem[]>([]);
	let libraryRows = $state<{ lib: Library; items: CatalogItem[] }[]>([]);
	let catalogTotal = $state(0);

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

			const active = sortByRecent(progressList.filter(isInProgress)).slice(0, 12);
			progressMap = new Map(active.map((p) => [p.item_id, p]));
			const activeItems = (await ensureItems(active.map((p) => p.item_id))).filter(
				(item): item is CatalogItem => item !== undefined
			);
			continueItems = activeItems;

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
		void scan.refresh().then(() => {
			if (scan.running) scan.follow();
		});
	});

	onDestroy(() => scan.stop());

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
			<Skeleton class="h-[72svh] min-h-[32rem] w-full rounded-none" />
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
		{@const upNext = hero ? (continueItems.find((i) => i.id !== hero.id) ?? null) : null}
		{#if hero}
			<Hero
				item={hero}
				enrichment={enrichmentCache.get(hero.id) ?? null}
				resume={progressMap.has(hero.id)}
				{upNext}
				upNextEnrichment={upNext ? enrichmentCache.get(upNext.id) ?? null : null}
				upNextProgress={upNext ? progressMap.get(upNext.id) ?? null : null}
			/>
		{/if}
		<section id="home-library" class="home-library relative bg-background pb-24 pt-14 sm:pt-18 md:pb-20 md:pt-20">
			<div class="mx-auto w-full max-w-[1800px] space-y-14 md:space-y-16 px-5 sm:px-8 lg:px-10">
				{#if scan.running}
					<div class="flex"><Badge tone="accent"><ScanLine class="size-3" /> Scanning…</Badge></div>
				{:else if scan.status?.state === 'error'}
					<div class="flex"><Badge tone="danger">Last scan failed</Badge></div>
				{/if}

				{#if continueItems.length > 0}
					<MediaRow title="Continue watching" href="/library" actionLabel="Open library" items={continueItems} {progressMap} layout="landscape" />
				{/if}

				{#if recent.length > 0}
					<MediaRow title="Recently added" href="/library" items={recent.slice(0, 12)} {progressMap} layout="landscape" />
				{/if}

				{#each libraryRows as row (row.lib.id)}
					<MediaRow title={row.lib.name} href={`/library/${row.lib.id}`} items={row.items} {progressMap} layout={row.lib.type === 'movie' ? 'poster' : 'landscape'} />
				{/each}
			</div>
		</section>
	{/if}
</div>
