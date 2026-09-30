<script lang="ts">
	import { onMount } from 'svelte';
	import Play from '@lucide/svelte/icons/play';
	import Info from '@lucide/svelte/icons/info';
	import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { mediaSubtitle } from '$lib/utilities/format';
	import { progressRatio } from '$lib/utilities/progress';
	import ProgressBar from './ProgressBar.svelte';
	import { loadPreferredPlayer, playbackHref, playExternalClick, type PreferredPlayer } from '$lib/player/external-player';
	import { isReadable } from '$lib/reader/kinds';
	let preferredPlayer = $state<PreferredPlayer>('browser');
	let origin = $state('');
	onMount(() => {
		origin = window.location.origin;
		if (session.user) preferredPlayer = loadPreferredPlayer(session.user.id);
	});
	function playHref(it: CatalogItem): string { return playbackHref(origin, it.id, preferredPlayer, it.kind); }
	// Comics and manga open in the reader, so no external player applies (D-085).
	function playerFor(it: CatalogItem): PreferredPlayer { return isReadable(it.kind) ? 'browser' : preferredPlayer; }

	let {
		item,
		enrichment = null,
		resume = false,
		upNext = null,
		upNextEnrichment = null,
		upNextProgress = null
	}: {
		item: CatalogItem;
		enrichment?: Enrichment | null;
		resume?: boolean;
		upNext?: CatalogItem | null;
		upNextEnrichment?: Enrichment | null;
		upNextProgress?: Progress | null;
	} = $props();

	const reading = $derived(isReadable(item.kind));
	// A page is portrait: shown as a cover over a blurred backdrop instead of
	// being stretched across the hero like a video still.
	const cover = $derived(api.thumbnail.url(item.id, session.token, { width: 720 }));
	const artwork = $derived(
		reading
			? enrichment?.cover || enrichment?.poster || cover
			: enrichment?.cover || enrichment?.poster || api.thumbnail.url(item.id, session.token, { width: 1600 })
	);
	const title = $derived(enrichment?.title || item.title);
	const detail = $derived(mediaSubtitle(item));
	const eyebrow = $derived(
		resume ? (reading ? 'Pick up where you left off' : 'Continue your story') : 'Now in your library'
	);
	const year = $derived(
		String(enrichment?.year || item.year || '')
	);
	const meta = $derived(
		[detail, (enrichment?.genres ?? []).slice(0, 2).join(' · ')]
			.filter(Boolean)
			.join('  /  ')
	);

	const upNextArt = $derived(
		upNext
			? upNextEnrichment?.cover ||
				upNextEnrichment?.poster ||
				api.thumbnail.url(upNext.id, session.token, { width: 320 })
			: ''
	);
	const upNextTitle = $derived(upNext ? upNextEnrichment?.title || upNext.title : '');
	const upNextRatio = $derived(
		upNextProgress && !upNextProgress.completed ? progressRatio(upNextProgress) : 0
	);
</script>

<section class="hero-monolith relative overflow-hidden bg-background" aria-label={title}>
	{#if reading}
		<img
			src={artwork}
			alt=""
			aria-hidden="true"
			decoding="async"
			referrerpolicy="no-referrer"
			class="absolute inset-0 size-full scale-125 object-cover opacity-30 blur-3xl"
		/>
		<img
			src={enrichment?.poster || cover}
			alt=""
			aria-hidden="true"
			loading="eager"
			fetchpriority="high"
			decoding="async"
			referrerpolicy="no-referrer"
			class="absolute right-[8%] top-[46%] hidden h-[64%] max-h-[38rem] w-auto -translate-y-1/2 rounded-xl object-contain shadow-[0_30px_80px_rgba(0,0,0,0.55)] ring-1 ring-white/12 md:block lg:right-[12%]"
		/>
	{:else if artwork}
		<img
			src={artwork}
			alt=""
			aria-hidden="true"
			loading="eager"
			fetchpriority="high"
			decoding="async"
			referrerpolicy="no-referrer"
			class="hero-monolith-art absolute inset-y-0 right-0 h-full w-full object-cover object-center md:w-[70%]"
		/>
	{/if}
	<div class="hero-monolith-shade absolute inset-0" aria-hidden="true"></div>
	<div class="relative mx-auto flex min-h-[calc(100svh-3.5rem)] w-full max-w-[1800px] flex-col justify-end px-5 sm:px-8 lg:px-10 pb-20 pt-28 md:min-h-[min(56rem,100svh)] md:justify-center md:pb-16 md:pt-28">
		<div class="hero-monolith-copy max-w-4xl">
			<p class="mb-4 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent sm:text-[11px]">
				{eyebrow}{#if year} / {year}{/if}
			</p>
			<h1 class="hero-monolith-title text-white">{title}</h1>
			{#if meta}
				<p class="mt-6 font-mono text-[10px] uppercase tracking-[0.13em] text-white/60 sm:text-xs">{meta}</p>
			{/if}
			{#if enrichment?.synopsis}
				<p class="mt-5 line-clamp-3 max-w-xl text-sm leading-6 text-white/68 sm:text-base sm:leading-7">
					{enrichment.synopsis}
				</p>
			{/if}
			<div class="mt-7 flex flex-wrap gap-3">
				<a
					href={playHref(item)}
					onclick={(e) => playExternalClick(e, item.id, playerFor(item))}
					class="inline-flex min-h-11 items-center gap-2 rounded-full bg-white px-6 text-sm font-bold text-black transition duration-300 ease-out hover:-translate-y-0.5 hover:bg-white/88 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white"
				>
					<Play class="size-4 fill-current" aria-hidden="true" />
					{isReadable(item.kind) ? (resume ? 'Continue reading' : 'Read') : resume ? 'Continue watching' : 'Play'}
				</a>
				<a
					href={`/item/${item.id}`}
					class="inline-flex min-h-11 items-center gap-2 rounded-full border border-white/16 bg-black/20 px-5 text-sm font-semibold text-white backdrop-blur-md transition duration-300 ease-out hover:-translate-y-0.5 hover:bg-white/10"
				>
					<Info class="size-4" aria-hidden="true" /> More details
				</a>
			</div>
		</div>
	</div>
	{#if upNext}
		<a
			href={playHref(upNext)}
			onclick={(e) => playExternalClick(e, upNext.id, playerFor(upNext))}
			class="absolute bottom-8 right-8 z-10 hidden w-[min(26rem,32vw)] grid-cols-[7rem_1fr] items-stretch border border-white/10 bg-black/75 backdrop-blur-xl transition duration-300 ease-out hover:-translate-y-0.5 hover:border-white/25 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white md:grid"
			aria-label={`Continue next: ${upNextTitle}`}
		>
			<div class="aspect-[7/6] overflow-hidden bg-surface">
				<img
					src={upNextArt}
					alt=""
					aria-hidden="true"
					loading="lazy"
					decoding="async"
					class="size-full object-cover"
				/>
			</div>
			<div class="min-w-0 p-4">
				<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent">Continue next</p>
				<h3 class="mt-2 truncate text-sm font-semibold text-white">{upNextTitle}</h3>
				{#if upNextRatio > 0}
					<ProgressBar ratio={upNextRatio} class="mt-3" />
				{/if}
			</div>
		</a>
	{/if}
</section>
