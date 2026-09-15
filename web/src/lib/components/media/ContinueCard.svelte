<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { mediaSubtitle } from '$lib/utilities/format';
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
	const epLabel = $derived(
		item.season > 0 && item.episode > 0
			? `S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`
			: item.episode > 0
				? `Episode ${item.episode}`
				: series
	);
	const title = $derived(item.episode > 0 ? `Episode ${item.episode}` : series);
	const subtitle = $derived(
		item.episode > 0 ? `${epLabel} - ${series}` : mediaSubtitle(item) || series
	);
	const ratio = $derived(progress && !progress.completed ? progressRatio(progress) : 0);

	let stillFailed = $state(false);
</script>

<a
	href={`/item/${item.id}`}
	class="group block transition-transform duration-300 ease-out hover:-translate-y-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={`${title}, ${subtitle}`}
>
	<div class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.18)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/14">
		<div class="aspect-video">
			{#if !stillFailed}
				<img
					src={still}
					alt=""
					aria-hidden="true"
					loading="lazy"
					decoding="async"
					class="size-full object-cover transition-transform duration-500 ease-out group-hover:scale-[1.035]"
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
				<ProgressBar {ratio} class="bg-black/50" />
			</div>
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="truncate text-sm font-semibold tracking-[-0.015em] text-foreground">{title}</p>
		<p class="mt-1 truncate text-xs text-muted">{subtitle}</p>
	</div>
</a>
