<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import Clapperboard from '@lucide/svelte/icons/clapperboard';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import ScanLine from '@lucide/svelte/icons/scan-line';
	import type { CatalogItem, Library, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import MediaGrid from '$lib/components/media/MediaGrid.svelte';
	import MediaRow from '$lib/components/media/MediaRow.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { ensureEnrichments, ensureItems, ensureLibraries } from '$lib/stores/media-cache.svelte';
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

<div class="space-y-8">
	<header class="flex flex-wrap items-center justify-between gap-3">
		<div>
			<h1 class="text-xl font-semibold tracking-tight text-foreground">Home</h1>
			<p class="mt-0.5 text-sm text-muted">
				{catalogTotal} {catalogTotal === 1 ? 'item' : 'items'} in your libraries
			</p>
		</div>
		{#if scan.running}
			<Badge tone="accent"><ScanLine class="size-3" /> Scanning…</Badge>
		{:else if scan.status?.state === 'error'}
			<Badge tone="danger">Last scan failed</Badge>
		{/if}
	</header>

	{#if loading}
		<div class="space-y-8">
			<div class="space-y-3">
				<Skeleton class="h-5 w-40" />
				<div class="flex gap-3 overflow-hidden">
					{#each Array(6) as _}
						<Skeleton class="h-52 w-36 shrink-0 sm:w-40" />
					{/each}
				</div>
			</div>
			<div class="space-y-3">
				<Skeleton class="h-5 w-32" />
				<div class="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-5 xl:grid-cols-6">
					{#each Array(6) as _}
						<Skeleton class="h-56" />
					{/each}
				</div>
			</div>
		</div>
	{:else if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if libraries.length === 0}
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
	{:else if catalogTotal === 0}
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
	{:else}
		{#if continueItems.length > 0}
			<MediaRow title="Continue watching" items={continueItems} {progressMap} />
		{/if}

		{#if recent.length > 0}
			<section class="space-y-3">
				<h2 class="text-base font-semibold tracking-tight text-foreground">Recently added</h2>
				<MediaGrid items={recent.slice(0, 12)} {progressMap} priority />
			</section>
		{/if}

		{#each libraryRows as row (row.lib.id)}
			<MediaRow
				title={row.lib.name}
				href={`/library/${row.lib.id}`}
				items={row.items}
				{progressMap}
			/>
		{/each}
	{/if}
</div>
