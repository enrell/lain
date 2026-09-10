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
</script>

<a
	href={`/item/${item.id}`}
	class="group block rounded-card focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={`${title}${subtitle ? `, ${subtitle}` : ''}`}
>
	<div
		class="relative overflow-hidden rounded-card border border-line/70 bg-surface transition-colors duration-150 group-hover:border-muted/40"
	>
		<div class="aspect-[2/3]">
			<Poster {item} {enrichment} {priority} class="size-full" />
		</div>
		<div
			class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/75 via-black/10 to-transparent opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100"
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
	<div class="mt-2 min-w-0">
		<p class="line-clamp-2 text-sm font-medium leading-snug text-foreground">{title}</p>
		{#if subtitle}
			<p class="mt-0.5 truncate text-xs text-muted">{subtitle}</p>
		{/if}
	</div>
</a>
