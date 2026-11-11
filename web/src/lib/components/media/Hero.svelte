<script lang="ts">
	import { onMount } from 'svelte';
	import Play from '@lucide/svelte/icons/play';
	import Info from '@lucide/svelte/icons/info';
	import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { formatLength, mediaSubtitle } from '$lib/utilities/format';
	import { t } from '$lib/i18n';
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
		progress = null
	}: {
		item: CatalogItem;
		enrichment?: Enrichment | null;
		resume?: boolean;
		progress?: Progress | null;
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
	const eyebrow = $derived(
		resume ? (reading ? 'Continue reading' : 'Continue watching') : 'New in your library'
	);
	// What it is, in words: the episode you are on and what is left of it,
	// or the year; then up to two genres.
	const left = $derived(
		resume && !reading && progress && progress.duration_sec - progress.position_sec > 30
			? t('title.left', { time: formatLength(progress.duration_sec - progress.position_sec) })
			: ''
	);
	const unit = $derived(
		reading ? mediaSubtitle(item).split(' · ')[0] : item.episode > 0 ? (item.season > 0 ? `Season ${item.season} · Episode ${item.episode}` : `Episode ${item.episode}`) : ''
	);
	const year = $derived(String(enrichment?.year || item.year || ''));
	const meta = $derived(
		[unit && unit !== String(item.year) ? unit : '', left, resume ? '' : year, ...(enrichment?.genres ?? []).slice(0, 2)].filter(Boolean)
	);
	const ratio = $derived(resume && progress && !progress.completed ? progressRatio(progress) : 0);
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
			class="hero-monolith-art absolute inset-0 hidden size-full object-cover object-center md:block"
		/>
		<!-- Phone: the art's colours wash the background and the art itself is
		     shown whole below, so a poster's own lettering never sits under
		     the title. -->
		<img
			src={artwork}
			alt=""
			aria-hidden="true"
			decoding="async"
			referrerpolicy="no-referrer"
			class="absolute inset-0 size-full scale-150 object-cover opacity-55 blur-3xl md:hidden"
		/>
	{/if}
	<div class="hero-monolith-shade absolute inset-0" aria-hidden="true"></div>
	<div class="relative mx-auto flex min-h-[calc(100svh-3.5rem)] w-full max-w-[1800px] flex-col justify-end px-5 sm:px-8 lg:px-10 pb-8 pt-8 max-md:min-h-0 md:min-h-[min(40rem,60svh)] md:justify-end md:pb-10 md:pt-36">
		<div class="hero-monolith-copy max-w-4xl">
			<img
				src={enrichment?.poster || cover}
				alt=""
				aria-hidden="true"
				loading="eager"
				decoding="async"
				referrerpolicy="no-referrer"
				class="mb-5 max-h-40 w-auto max-w-full rounded-xl object-contain shadow-[0_20px_50px_rgba(0,0,0,0.5)] ring-1 ring-white/10 md:hidden"
			/>
			<p class="mb-4 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent sm:text-[11px]">
				{eyebrow}
			</p>
			<h1 class="hero-monolith-title text-white">{title}</h1>
			{#if meta.length > 0}
				<ul class="mt-5 flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-white/75" aria-label="About">
					{#each meta as part, i (part)}
						<li class="flex items-center gap-3">{#if i > 0}<span class="size-1 rounded-full bg-white/35" aria-hidden="true"></span>{/if}{part}</li>
					{/each}
				</ul>
			{/if}
			{#if ratio > 0}
				<ProgressBar {ratio} class="mt-4 max-w-xs" />
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
					{#if resume}
						Continue{#if !reading && item.episode > 0}<span class="font-semibold opacity-60">· E{item.episode}</span>{/if}
					{:else}
						{isReadable(item.kind) ? 'Read' : 'Play'}
					{/if}
				</a>
				<a
					href={`/item/${item.id}`}
					class="inline-flex min-h-11 items-center gap-2 rounded-full border border-white/16 bg-black/20 px-5 text-sm font-semibold text-white backdrop-blur-md transition duration-300 ease-out hover:-translate-y-0.5 hover:bg-white/10"
				>
					<Info class="size-4" aria-hidden="true" /> Details
				</a>
			</div>
		</div>
	</div>
</section>
