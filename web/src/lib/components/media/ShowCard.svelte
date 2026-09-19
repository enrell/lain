<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import type { Progress } from '$lib/api/types';
	import type { SeriesGroup } from '$lib/utilities/grouping';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import Poster from './Poster.svelte';

	/**
	 * Plex-style library card: one poster per show, never one per
	 * episode. The representative item is the first episode that
	 * already has artwork, so a partially enriched show still shows
	 * its poster instead of a lone frame. The card opens the show's own
	 * page, which lists the episodes.
	 */
	let {
		group,
		progress = null,
		priority = false
	}: {
		group: SeriesGroup;
		progress?: Progress | null;
		priority?: boolean;
	} = $props();

	const artItem = $derived(group.items.find((i) => i.episode > 0) ?? group.items[0]);
	const seasons = $derived([...new Set(group.items.map((i) => i.season).filter((s) => s > 0))]);
	const detail = $derived.by(() => {
		const parts: string[] = [`${group.count} episodes`];
		if (seasons.length === 1) parts.push(`Season ${seasons[0]}`);
		else if (seasons.length > 1) parts.push(`Seasons ${Math.min(...seasons)}–${Math.max(...seasons)}`);
		if (group.year > 0) parts.push(String(group.year));
		return parts.join(' · ');
	});
</script>

<a
	href={`/item/${artItem.id}`}
	class="group block w-full text-left transition-transform duration-300 ease-out hover:-translate-y-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={`${group.title}, ${detail}`}
>
	<div
		class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.16)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/15"
	>
		<div class="aspect-[2/3]">
			<Poster
				item={artItem}
				enrichment={enrichmentCache.get(artItem.id) ?? null}
				{priority}
				class="size-full transition-transform duration-500 ease-out group-hover:scale-[1.035]"
			/>
		</div>
		<div
			class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/80 via-black/10 to-transparent opacity-15 transition-opacity duration-300 group-hover:opacity-100"
		></div>
		<div
			class="pointer-events-none absolute right-2 top-2 flex size-9 items-center justify-center rounded-full bg-black/65 text-foreground opacity-0 backdrop-blur transition-opacity duration-300 group-hover:opacity-100"
		>
			<Play class="size-4 translate-x-px" aria-hidden="true" />
		</div>
		{#if progress && progress.duration_sec > 0}
			<div class="absolute inset-x-0 bottom-0 h-1 bg-black/40">
				<div
					class="h-full bg-accent"
					style={`width: ${Math.min(100, Math.round((progress.position_sec / progress.duration_sec) * 100))}%`}
				></div>
			</div>
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="line-clamp-2 text-sm font-semibold leading-snug tracking-[-0.015em] text-foreground">{group.title}</p>
		<p class="mt-1 truncate text-xs text-muted">{detail}</p>
	</div>
</a>
