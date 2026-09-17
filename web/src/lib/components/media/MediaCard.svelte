<script lang="ts">
	import Play from '@lucide/svelte/icons/play';
	import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
	import { mediaSubtitle } from '$lib/utilities/format';
	import { progressRatio } from '$lib/utilities/progress';
	import Poster from './Poster.svelte';
	import ProgressBar from './ProgressBar.svelte';

	let {
		item,
		enrichment = null,
		progress = null,
		priority = false
	}: {
		item: CatalogItem;
		enrichment?: Enrichment | null;
		progress?: Progress | null;
		priority?: boolean;
	} = $props();

	const title = $derived(enrichment?.title || item.title);
	const subtitle = $derived(
		[enrichment?.year && item.year === 0 ? String(enrichment.year) : '', mediaSubtitle(item)]
			.filter(Boolean)
			.join(' · ')
	);
	const ratio = $derived(progress && !progress.completed ? progressRatio(progress) : 0);
	// The link's aria-label replaces its inner text, so the visual 'Watching'
	// badge has to be part of the name or a screen reader never hears it.
	const accessibleName = $derived(
		[title, subtitle, ratio > 0 ? 'watching' : ''].filter(Boolean).join(', ')
	);
</script>

<a
	href={`/item/${item.id}`}
	class="group block rounded-card transition-transform duration-300 ease-out hover:-translate-y-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={accessibleName}
>
	<div
		class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.16)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/15"
	>
		<div class="aspect-[2/3]">
			<Poster {item} {enrichment} {priority} class="size-full transition-transform duration-500 ease-out group-hover:scale-[1.035]" />
		</div>
		<div
			class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/80 via-black/10 to-transparent opacity-15 transition-opacity duration-300 group-hover:opacity-100 group-focus-visible:opacity-100"
		></div>
		<div
			class="pointer-events-none absolute right-2 top-2 flex size-9 items-center justify-center rounded-full bg-black/65 text-foreground opacity-0 backdrop-blur transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100"
		>
			<Play class="size-4 translate-x-px" aria-hidden="true" />
		</div>
		{#if ratio > 0}
			<span
				class="absolute left-2 top-2 rounded-full bg-black/65 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-foreground backdrop-blur"
			>
				Watching
			</span>
		{/if}
		{#if ratio > 0}
			<div class="absolute inset-x-0 bottom-0 p-1.5">
				<ProgressBar {ratio} label="Watch progress" class="bg-black/50" />
			</div>
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="line-clamp-2 text-sm font-semibold leading-snug tracking-[-0.015em] text-foreground">{title}</p>
		{#if subtitle}
			<p class="mt-1 truncate text-xs text-muted">{subtitle}</p>
		{/if}
	</div>
</a>
