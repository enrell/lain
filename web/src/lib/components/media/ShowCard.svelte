<script lang="ts">
	import { isReadable } from '$lib/reader/kinds';
	import Check from '@lucide/svelte/icons/check';
	import Play from '@lucide/svelte/icons/play';
	import type { Progress } from '$lib/api/types';
	import type { SeriesGroup } from '$lib/utilities/grouping';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import { artItem as pickArt } from '$lib/utilities/browse';
	import { readingLabel } from '$lib/reader/kinds';
	import { episodeLabel } from '$lib/utilities/grouping';
	import { formatLength, timeLeft } from '$lib/utilities/format';
	import { t } from '$lib/i18n';
	import CardProgress from './CardProgress.svelte';
	import type { Shape } from '$lib/stores/shapes.svelte';
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
		progressOf = undefined,
		priority = false,
		frame = undefined
	}: {
		group: SeriesGroup;
		progress?: Progress | null;
		/** Each part's progress: the card then marks the title as a whole. */
		progressOf?: (id: string) => Progress | undefined;
		priority?: boolean;
		/** Fill the container's width in the frame its grid or rail shares
		 * (see `guessFrame`); a picture of the other shape is contained. */
		frame?: Shape;
	} = $props();

	const artItem = $derived(pickArt(group));

	/*
	 * One rule for every card: a title under way says Watching (or Reading)
	 * with a strip of how much of the whole is done; a finished one says
	 * Watched (or Read) with a check and no strip; an untouched one says
	 * nothing. One episode's strip used to stand for the whole show.
	 */
	const reading = $derived(isReadable(group.items[0]?.kind));
	const state = $derived.by(() => {
		if (progressOf) {
			let done = 0;
			let part = 0;
			for (const it of group.items) {
				const p = progressOf(it.id);
				if (!p) continue;
				if (p.completed) done += 1;
				else if (p.duration_sec > 0 && p.position_sec > 0) part += Math.min(1, p.position_sec / p.duration_sec);
			}
			const n = Math.max(1, group.count);
			return { done, started: done > 0 || part > 0, finished: done >= n, fraction: (done + part) / n };
		}
		if (!progress || progress.duration_sec <= 0) return { done: 0, started: false, finished: false, fraction: 0 };
		return {
			done: progress.completed ? 1 : 0,
			started: true,
			finished: false,
			fraction: progress.completed ? 1 : progress.position_sec / progress.duration_sec
		};
	});

	// Begun with nothing finished yet: the part under way and how far into it.
	// "0 of 12 watched" under a Watching badge read as a contradiction.
	const current = $derived.by(() => {
		if (!progressOf || state.done > 0) return null;
		for (const it of group.items) {
			const p = progressOf(it.id);
			if (p && !p.completed && p.position_sec > 0 && p.duration_sec > 0) return { item: it, progress: p };
		}
		return null;
	});

	const seasons = $derived([...new Set(group.items.map((i) => i.season).filter((s) => s > 0))]);
	const detail = $derived.by(() => {
		const parts: string[] = [
			isReadable(group.items[0]?.kind)
				? `${group.count} ${group.count === 1 ? 'volume' : 'volumes'}`
				: `${group.count} episodes`
		];
		// A reading file's "season" is its volume; the count already says it.
		if (isReadable(group.items[0]?.kind)) {
			/* no season line */
		} else if (seasons.length === 1) parts.push(`Season ${seasons[0]}`);
		else if (seasons.length > 1) parts.push(`Seasons ${Math.min(...seasons)}–${Math.max(...seasons)}`);
		if (group.year > 0) parts.push(String(group.year));
		// Under way, the line says how far, as the app's cards do: the part and
		// what is left of it while none is finished, then "2 of 12 watched".
		if (progressOf && state.started && !state.finished) {
			if (current) {
				const { item, progress: p } = current;
				const part = reading ? readingLabel(item) : episodeLabel(item);
				if (group.count === 1) {
					return reading
						? t('browse.pageOf', { page: Math.max(1, Math.floor(p.position_sec)), pages: Math.floor(p.duration_sec) })
						: timeLeft(p.position_sec, p.duration_sec);
				}
				return reading
					? t('browse.partPage', { part, page: Math.max(1, Math.floor(p.position_sec)), pages: Math.floor(p.duration_sec) })
					: t('browse.partLeft', { part, time: formatLength(Math.max(1, p.duration_sec - p.position_sec)) });
			}
			return t(reading ? 'title.readOf' : 'title.watchedOf', { done: state.done, count: group.count });
		}
		return parts.join(' · ');
	});
</script>

<a
	href={`/item/${artItem.id}`}
	class="group block w-full shrink-0 text-left transition-transform duration-300 ease-out hover:-translate-y-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={`${group.title}, ${detail}`}
>
	<div
		class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.16)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/15"
	>
		<div class={frame === 'landscape' ? 'aspect-video' : 'aspect-[2/3]'}>
			<Poster
				item={artItem}
				enrichment={enrichmentCache.get(artItem.id) ?? null}
				{priority}
				{frame}
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
		{#if state.finished}
			<span class="absolute left-2 top-2 flex items-center gap-1 rounded-full bg-black/65 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-foreground backdrop-blur">
				<Check class="size-3" aria-hidden="true" />{reading ? 'Read' : 'Watched'}
			</span>
		{:else if state.started}
			<span class="absolute left-2 top-2 rounded-full bg-black/65 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-foreground backdrop-blur">
				{reading ? 'Reading' : 'Watching'}
			</span>
			<!-- The film cards' bar, inset and rounded: a flush strip here and an inset bar
			     there drew one fact two ways in the same grid. -->
			<CardProgress ratio={Math.min(1, state.fraction)} label={t(reading ? 'title.readProgress' : 'title.watchProgress')} />
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="line-clamp-2 text-sm font-semibold leading-snug tracking-[-0.015em] text-foreground">{group.title}</p>
		<p class="mt-1 truncate text-xs text-muted">{detail}</p>
	</div>
</a>
