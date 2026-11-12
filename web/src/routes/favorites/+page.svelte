<script lang="ts">
	/*
	 * The titles kept with the heart: the page the Favorites rail on Home
	 * opens. The list itself is the social module's (the one a profile
	 * shows); a server without that module has no favourites to list.
	 *
	 * It is a library browser over the favourites: one long unfiltered
	 * sequence stopped working once there were many, so they get the same
	 * search, sort, filters and tile size as a library, newest kept first.
	 */
	import Heart from '@lucide/svelte/icons/heart';
	import { api } from '$lib/api';
	import type { CatalogItem } from '$lib/api/types';
	import LibraryBrowser from '$lib/components/media/LibraryBrowser.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import { ensureItems } from '$lib/stores/media-cache.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { t } from '$lib/i18n';

	// Favourites whose files are no longer in a library this account can see.
	let gone = $state(0);

	/*
	 * Every file of each kept title, newest kept first: a favourite series is
	 * the series, not the episode the heart was pressed on, so its card counts
	 * and marks the whole of it.
	 */
	async function favourites(): Promise<CatalogItem[]> {
		let favorites;
		try {
			favorites = (await api.social.settings()).favorites;
		} catch (err) {
			throw new Error(errorMessage(err, t('favorites.loadFailed')));
		}
		const ids = favorites.map((f) => f.item_id ?? '').filter((id) => id !== '');
		const found = (await ensureItems([...ids].reverse())).filter((item): item is CatalogItem => item !== undefined);
		const parts = await Promise.all(
			found.map((item) =>
				api.catalog
					.episodes(item.id)
					.then((r) => (r.items.length > 0 ? r.items : [item]))
					.catch(() => [item])
			)
		);
		// Two episodes of one show kept apart are still one title.
		const seen = new Set<string>();
		const items: CatalogItem[] = [];
		for (const item of parts.flat()) {
			if (seen.has(item.id)) continue;
			seen.add(item.id);
			items.push(item);
		}
		gone = ids.length - found.length;
		return items;
	}
</script>

<svelte:head><title>{t('favorites.title')} — Lain</title></svelte:head>

<LibraryBrowser
	source={favourites}
	listed={t('favorites.sortListed')}
	prefsKey="lain.favorites.prefs"
	searchLabel={t('favorites.search')}
>
	{#snippet header()}
		<header class="space-y-1">
			<h1 class="page-title">{t('favorites.title')}</h1>
			<p class="text-sm text-muted">
				{t('favorites.lead')}{#if gone > 0}{' '}{t('favorites.gone', { count: gone })}{/if}
			</p>
		</header>
	{/snippet}
	{#snippet empty()}
		<EmptyState title={t('favorites.emptyTitle')} description={t('favorites.emptyBody')}>
			{#snippet icon()}<Heart class="size-6 text-muted" />{/snippet}
			<LinkButton href="/library">{t('favorites.browse')}</LinkButton>
		</EmptyState>
	{/snippet}
</LibraryBrowser>
