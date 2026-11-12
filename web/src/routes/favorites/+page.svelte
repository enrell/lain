<script lang="ts">
	/*
	 * The titles kept with the heart: the page the Favorites rail on Home
	 * opens. The list itself is the social module's (the one a profile
	 * shows); a server without that module has no favourites to list.
	 */
	import { onMount } from 'svelte';
	import Heart from '@lucide/svelte/icons/heart';
	import { api } from '$lib/api';
	import type { CatalogItem } from '$lib/api/types';
	import MediaCard from '$lib/components/media/MediaCard.svelte';
	import { FIT } from '$lib/components/media/MediaGrid.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { enrichmentCache, ensureEnrichments, ensureItems } from '$lib/stores/media-cache.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let items = $state<CatalogItem[]>([]);
	// Favourites whose files are no longer in a library this account can see.
	let gone = $state(0);
	let loading = $state(true);
	let error = $state<string | null>(null);

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const favorites = (await api.social.settings()).favorites;
			const ids = favorites.map((f) => f.item_id ?? '').filter((id) => id !== '');
			// Newest first, as the rail on Home shows them.
			const found = (await ensureItems(ids.reverse())).filter((item): item is CatalogItem => item !== undefined);
			await ensureEnrichments(found.map((i) => i.id));
			items = found;
			gone = favorites.length - found.length;
		} catch (err) {
			error = errorMessage(err, 'Could not load your favorites.');
		} finally {
			loading = false;
		}
	}

	onMount(() => void load());
</script>

<svelte:head><title>Favorites — Lain</title></svelte:head>

<div class="space-y-6">
	<header class="space-y-1">
		<h1 class="text-xl font-semibold tracking-tight text-foreground">Favorites</h1>
		<p class="text-sm text-muted">
			{#if loading}
				The titles you kept with the heart.
			{:else}
				{items.length} {items.length === 1 ? 'title' : 'titles'} kept with the heart{#if gone > 0}; {gone} more {gone === 1 ? 'is' : 'are'} no longer in a library you can see{/if}.
			{/if}
		</p>
	</header>

	{#if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if loading}
		<div class="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
			{#each Array(6) as _}
				<Skeleton class="h-56" />
			{/each}
		</div>
	{:else if items.length === 0}
		<EmptyState title="No favorites yet" description="The heart beside a title's main action keeps it here and on Home.">
			{#snippet icon()}<Heart class="size-6 text-muted" />{/snippet}
			<LinkButton href="/library">Browse the library</LinkButton>
		</EmptyState>
	{:else}
		<div class={FIT}>
			{#each items as item, i (item.id)}
				<MediaCard {item} enrichment={enrichmentCache.get(item.id) ?? null} priority={i < 6} fit />
			{/each}
		</div>
	{/if}
</div>
