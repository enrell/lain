<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import { formatLength, mediaSubtitle } from '$lib/utilities/format';
	import { isReadable, readingLabel } from '$lib/reader/kinds';
	import { progressRatio } from '$lib/utilities/progress';
	import ProgressBar from './ProgressBar.svelte';

	let {
		item,
		enrichment = null,
		progress = null
	}: {
		item: CatalogItem;
		enrichment?: Enrichment | null;
		progress?: Progress | null;
	} = $props();

	// Landscape continue-watching card: episode still, progress rail,
	// episode label over the series title.
	const still = $derived(api.thumbnail.url(item.id, session.token, { width: 640 }));
	const series = $derived(enrichment?.title || item.title);
	const reading = $derived(isReadable(item.kind));
	const epLabel = $derived(
		reading
			? readingLabel(item)
			: item.season > 0 && item.episode > 0
			? `S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`
			: item.episode > 0
				? `Episode ${item.episode}`
				: series
	);
	// The title names the work and the line under it says where you are in
	// it and what is left, each said once: "Paper Moon Station" over
	// "Episode 3 · 9 sec left", as the app's cards read.
	const left = $derived(
		progress && !progress.completed && progress.duration_sec - progress.position_sec > 0
			? reading
				? `Page ${Math.max(1, Math.floor(progress.position_sec))} of ${Math.floor(progress.duration_sec)}`
				: t('title.left', { time: formatLength(progress.duration_sec - progress.position_sec) })
			: ''
	);
	const title = $derived(series);
	const subtitle = $derived(
		[reading ? (epLabel !== series ? epLabel : '') : item.episode > 0 ? epLabel : '', left || (item.episode > 0 || reading ? '' : mediaSubtitle(item))]
			.filter(Boolean)
			.join(' · ') || mediaSubtitle(item)
	);
	const ratio = $derived(progress && !progress.completed ? progressRatio(progress) : 0);

	let stillFailed = $state(false);
	// An upright picture (a file that came with its poster, a page) is shown
	// whole over a blurred copy of itself, as a grid's frame treats it; cut
	// to 16:9 a poster lost its title.
	let upright = $state(false);
</script>

<a
	href={`/item/${item.id}`}
	class="group block transition-transform duration-300 ease-out hover:-translate-y-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={`${title}, ${subtitle}${ratio > 0 ? (reading ? ', reading' : ', watching') : ''}`}
>
	<div class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.18)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/14">
		<div class="relative aspect-video">
			{#if !stillFailed}
				{#if upright}
					<img src={still} alt="" aria-hidden="true" decoding="async" class="absolute inset-0 size-full scale-125 object-cover opacity-50 blur-2xl" />
				{/if}
				<img
					src={still}
					alt=""
					aria-hidden="true"
					loading="lazy"
					decoding="async"
					class="relative size-full {upright ? 'object-contain' : 'object-cover'} {reading && !upright ? 'object-top' : ''} transition-transform duration-500 ease-out group-hover:scale-[1.035]"
					onload={(e) => {
						const img = e.currentTarget as HTMLImageElement;
						upright = img.naturalWidth > 0 && img.naturalWidth <= img.naturalHeight * 1.15;
					}}
					onerror={() => (stillFailed = true)}
				/>
			{/if}
		</div>
		<div
			class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/80 via-black/5 to-transparent opacity-30 transition-opacity duration-300 group-hover:opacity-100 group-focus-visible:opacity-100"
		></div>
		<div
			class="pointer-events-none absolute right-2 top-2 flex size-9 items-center justify-center rounded-full bg-black/65 text-foreground opacity-0 backdrop-blur transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100"
		>
			<Play class="size-4 translate-x-px" aria-hidden="true" />
		</div>
		{#if ratio > 0}
			<div class="absolute inset-x-0 bottom-0 p-1.5">
				<ProgressBar {ratio} label="Watch progress" class="bg-black/50" />
			</div>
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="truncate text-sm font-semibold tracking-[-0.015em] text-foreground">{title}</p>
		<p class="mt-1 truncate text-xs text-muted">{subtitle}</p>
	</div>
</a>
