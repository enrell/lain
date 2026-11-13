<script lang="ts">
	/*
	 * Where to go from a title: the other titles of its library, as one
	 * rail under the page. Fetched once per title; a failure or an empty
	 * library shows nothing. With `recent`, the newest titles of every
	 * library instead (Search before anything is typed); with `source`,
	 * whatever files it gives, in its order (what friends watch).
	 */
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { CatalogItem, Progress } from '$lib/api/types';
	import { t } from '$lib/i18n';
	import { guessFrame, settleFrame, shapesNow } from '$lib/stores/frames';
	import { enrichmentCache, ensureLibraries, libraryName } from '$lib/stores/media-cache.svelte';
	import { splitByShape, type Shape } from '$lib/stores/shapes.svelte';
	import { listTitles } from '$lib/stores/titles';
	import { groupItems, type SeriesGroup } from '$lib/utilities/grouping';
	import MediaCard from './MediaCard.svelte';
	import { RAIL } from './grid';
	import Rail from './Rail.svelte';
	import ShowCard from './ShowCard.svelte';

	const TITLES = 12;

	let {
		libraryId = '',
		exclude = [],
		heading,
		recent = false,
		source = undefined,
		class: className = 'mt-12'
	}: {
		libraryId?: string;
		exclude?: string[];
		heading: string;
		recent?: boolean;
		source?: () => Promise<CatalogItem[]>;
		class?: string;
	} = $props();

	/*
	 * The rail in Home's sizes for its frame, as the library's shelf has:
	 * guessed when it is drawn, checked once against its pictures and then
	 * kept (see `guessFrame`). Every library's newest titles mix posters and
	 * wide stills (films beside clips): they are two rails, each in its own
	 * frame, the second named by its libraries, rather than one shape boxed
	 * in the other's frame.
	 */
	let rails = $state<{ shape: Shape; groups: SeriesGroup[]; heading: string }[]>([]);
	// The account's positions, so these cards say watched or under way as the
	// same titles do on the library's shelves; without them a rail of bare
	// posters read as titles never opened.
	let progressMap = $state<Map<string, Progress>>(new Map());
	onMount(() => {
		void api.me
			.continueWatching()
			.then((list) => (progressMap = new Map(list.map((p) => [p.item_id, p]))))
			.catch(() => {});
	});
	// Only the latest load may draw or correct: measuring takes up to a few
	// seconds, so moving on to the next title of the same library could
	// otherwise let the previous title's rail (which lists the new title)
	// land after its own.
	let run = 0;

	/** The rails a set makes: by shape across libraries, one frame within one. */
	function railsOf(found: SeriesGroup[], shapes: Map<string, Shape>, split: boolean) {
		if (!split) {
			const portrait = found.filter((g) => shapes.get(g.key) !== 'landscape').length;
			return [{ shape: (portrait * 2 >= found.length ? 'portrait' : 'landscape') as Shape, groups: found, heading }];
		}
		return splitByShape(found, (g) => shapes.get(g.key) ?? 'portrait').map((part, i) => {
			if (i === 0) return { shape: part.shape, groups: part.items, heading };
			const names = [...new Set(part.items.flatMap((g) => g.libraryIds).map((id) => libraryName(id) ?? ''))].filter(Boolean);
			return { shape: part.shape, groups: part.items, heading: names.length ? t('search.newestIn', { names: names.join(', ') }) : heading };
		});
	}

	$effect(() => {
		const id = libraryId;
		const skip = new Set(exclude);
		const newest = recent;
		const given = source;
		const mine = ++run;
		rails = [];
		if (!id && !newest && !given) return;
		// The titles whole (D-283); one more than shown, for the one this page is.
		const titles = given
			? given().then((items) => groupItems(items))
			: listTitles(newest ? { sort: 'recent', limit: TITLES } : { libraryId: id, limit: TITLES + 1 }).then((p) => p.groups);
		void titles
			.then(async (all) => {
				const found = all.filter((g) => !g.items.some((i) => skip.has(i.id))).slice(0, TITLES);
				if (newest) await ensureLibraries();
				if (mine !== run) return;
				await guessFrame(found);
				if (mine !== run) return;
				const guess = shapesNow(found);
				rails = railsOf(found, guess, newest);
				await settleFrame(found);
				if (mine !== run) return;
				const settled = shapesNow(found);
				if ([...settled].some(([k, shape]) => guess.get(k) !== shape)) rails = railsOf(found, settled, newest);
			})
			.catch(() => {});
	});
</script>

{#each rails as rail, r (rail.shape)}
	<section class={['space-y-4', r === 0 ? className : 'mt-12'].join(' ')} aria-label={rail.heading}>
		<h2 class="section-title">{rail.heading}</h2>
		<!-- Through the gutter to the window's edge, as MediaRow's rails. -->
		<Rail class={RAIL[rail.shape]}>
			{#each rail.groups as group (group.key)}
				<div class="w-[var(--card-w)] shrink-0 snap-start">
					{#if group.count > 1}
						<ShowCard {group} frame={rail.shape} progressOf={(id) => progressMap.get(id)} />
					{:else}
						<MediaCard
							item={group.items[0]}
							enrichment={enrichmentCache.get(group.items[0].id) ?? null}
							progress={progressMap.get(group.items[0].id) ?? null}
							frame={rail.shape}
						/>
					{/if}
				</div>
			{/each}
		</Rail>
	</section>
{/each}
