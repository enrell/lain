<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import { wideStill } from '$lib/stores/frames';
	import { toldShape } from '$lib/stores/shapes.svelte';
	import { mediaSubtitle, timeLeft } from '$lib/utilities/format';
	import { isReadable, readingLabel } from '$lib/reader/kinds';
	import { progressRatio } from '$lib/utilities/progress';
	import CardProgress from './CardProgress.svelte';
	import OfflineMark from './OfflineMark.svelte';
	import PlaceholderPoster from './PlaceholderPoster.svelte';

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
	const series = $derived(enrichment?.title || item.title);
	const reading = $derived(isReadable(item.kind));
	// A video shows a wide picture of itself in this wide card: an episode
	// its own still, never the show's poster kept beside its first episode
	// (see `wideStill`), and a film its overlay's backdrop or else its own
	// still, where its poster stood pillarboxed between blurred bars. A book
	// keeps its cover.
	const framed = $derived(!reading);
	const still = $derived(
		reading
			? api.thumbnail.url(item.id, session.token, { width: 640 })
			: item.episode <= 0 && enrichment?.cover
				? enrichment.cover
				: wideStill(item, 640)
	);
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
				: timeLeft(progress.position_sec, progress.duration_sec)
			: ''
	);
	const title = $derived(series);
	const subtitle = $derived(
		[reading ? (epLabel !== series ? epLabel : '') : item.episode > 0 ? epLabel : '', left || (item.episode > 0 || reading ? '' : mediaSubtitle(item))]
			.filter(Boolean)
			.join(' · ') || mediaSubtitle(item)
	);
	const ratio = $derived(progress && !progress.completed ? progressRatio(progress) : 0);

	// D-287: never an empty box. The file's still, then the title's art from
	// its overlay (wide first), then the lettered placeholder. A server since
	// D-287 already answers the title's art for a file it cannot reach; this
	// chain covers an older server, a broken picture and a provider that is
	// down. Pictures that failed are remembered by address, so another item
	// starts its own chain afresh.
	const chain = $derived(
		[still, enrichment?.cover, enrichment?.poster].filter((s, i, all): s is string => !!s && all.indexOf(s) === i)
	);
	let failed = $state<string[]>([]);
	const shown = $derived(chain.find((s) => !failed.includes(s)) ?? '');
	const placeholderLabel = $derived(item.episode > 0 && !reading ? epLabel : '');
	const offline = $derived(!!item.offline);
	// An upright picture (a file that came with its poster, a page) is shown
	// whole over a blurred copy of itself, as a grid's frame treats it; cut
	// to 16:9 a poster lost its title. Known before it loads when the server
	// says (D-282), so the card is drawn that way at once; else measured.
	let measuredUpright = $state<boolean | null>(null);
	const upright = $derived(measuredUpright ?? (!framed && toldShape(item) === 'portrait'));
</script>

<a
	href={`/item/${item.id}`}
	class="group block transition-transform duration-300 ease-out hover:-translate-y-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={`${offline ? t('unreachable.cardLabel', { title }) : title}, ${subtitle}${ratio > 0 ? (reading ? ', reading' : ', watching') : ''}`}
>
	<div class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.18)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/14">
		<div class="relative aspect-video">
			{#if shown}
				{#if upright}
					<img src={shown} alt="" aria-hidden="true" decoding="async" referrerpolicy="no-referrer" class="absolute inset-0 size-full scale-125 object-cover opacity-50 blur-2xl" />
				{/if}
				<img
					src={shown}
					alt=""
					aria-hidden="true"
					loading="lazy"
					decoding="async"
					referrerpolicy="no-referrer"
					class="relative size-full {upright ? 'object-contain' : 'object-cover'} {reading && !upright ? 'object-top' : ''} transition-transform duration-500 ease-out group-hover:scale-[1.035]"
					onload={(e) => {
						const img = e.currentTarget as HTMLImageElement;
						measuredUpright = img.naturalWidth > 0 && img.naturalWidth <= img.naturalHeight * 1.15;
					}}
					onerror={() => {
						failed = [...failed, shown];
						measuredUpright = null;
					}}
				/>
			{:else}
				<PlaceholderPoster title={series} label={placeholderLabel} wide class="size-full" />
			{/if}
		</div>
		<div
			class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/80 via-black/5 to-transparent opacity-30 transition-opacity duration-300 group-hover:opacity-100 group-focus-visible:opacity-100"
		></div>
		{#if offline}
			<OfflineMark class="absolute right-2 top-2" />
		{:else}
			<div
				class="pointer-events-none absolute right-2 top-2 flex size-9 items-center justify-center rounded-full bg-black/65 text-foreground opacity-0 backdrop-blur transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100"
			>
				<Play class="size-4 translate-x-px" aria-hidden="true" />
			</div>
		{/if}
		{#if ratio > 0}
			<CardProgress {ratio} label={t('title.watchProgress')} />
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="truncate text-sm font-semibold tracking-[-0.015em] text-foreground">{title}</p>
		<p class="mt-1 truncate text-xs text-muted">{subtitle}</p>
	</div>
</a>
