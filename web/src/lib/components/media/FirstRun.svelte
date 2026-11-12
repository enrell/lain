<script lang="ts">
	/*
	 * What a page is for, before it has anything of yours to show: an
	 * accent eyebrow, a large tight line and one or two of plain text, the
	 * first step as the main action, and art from this server's own
	 * library, as Home's hero opens. Not a box: the art's colour washes the
	 * page behind the words and fades out at every edge.
	 *
	 * Steps, when given, sit under a hairline as short mono-labelled notes,
	 * the way a rail's heading sits over its cards.
	 */
	import { onMount, type Snippet } from 'svelte';
	import { api } from '$lib/api';
	import type { CatalogItem } from '$lib/api/types';
	import { cardPicture } from '$lib/stores/frames';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { guessShape, measure, shapeOf } from '$lib/stores/shapes.svelte';
	import { artItem } from '$lib/utilities/browse';
	import { groupItems } from '$lib/utilities/grouping';
	import Poster from './Poster.svelte';

	let {
		eyebrow,
		title,
		body,
		steps = [],
		art = true,
		children
	}: {
		eyebrow: string;
		title: string;
		body: string;
		/** Short notes under the actions: a mono label and one line each. */
		steps?: { label: string; text: string }[];
		/** Show three titles from the library beside the words. */
		art?: boolean;
		/** The actions: the first step first. */
		children?: Snippet;
	} = $props();

	const FAN = 3;
	let fan = $state<CatalogItem[]>([]);

	// The newest titles that stand upright, so the fan reads as posters; a
	// wide still fills in only when there are too few.
	onMount(async () => {
		if (!art) return;
		try {
			const page = await api.catalog.list({ sort: 'recent', limit: 60 });
			const picks = groupItems(page.items).slice(0, 12).map(artItem);
			await ensureEnrichments(picks.map((i) => i.id));
			await measure(picks.map((i) => ({ id: i.id, src: cardPicture(i, enrichmentCache.get(i.id)) })), 1500);
			const upright = (i: CatalogItem) => shapeOf(i.id, guessShape(i, enrichmentCache.get(i.id))) === 'portrait';
			fan = [...picks.filter(upright), ...picks.filter((i) => !upright(i))].slice(0, FAN);
		} catch {
			// Decoration: without it the words stand alone.
		}
	});

	const lead = $derived(fan[1] ?? fan[0]);
</script>

<section class="relative isolate">
	{#if lead}
		<!-- The middle poster's colours behind everything, faded out on every side. -->
		<div
			class="pointer-events-none absolute -top-16 -right-10 -z-10 h-[calc(100%+8rem)] w-[calc(100%+5rem)] overflow-hidden opacity-45 [mask-image:radial-gradient(closest-side,black_15%,transparent)] md:w-[75%]"
			aria-hidden="true"
		>
			<Poster item={lead} enrichment={enrichmentCache.get(lead.id) ?? null} class="size-full scale-110 blur-3xl" />
		</div>
	{/if}

	<div class="grid items-center gap-10 md:grid-cols-[minmax(0,1fr)_auto] lg:gap-16">
		<div class="min-w-0 py-2 md:py-8">
			<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent sm:text-[11px]">{eyebrow}</p>
			<h2 class="mt-4 max-w-[18ch] text-[2rem] font-semibold leading-[1.04] tracking-[-0.035em] text-balance text-foreground sm:text-5xl sm:leading-[1.02]">
				{title}
			</h2>
			<p class="mt-4 max-w-xl text-sm leading-6 text-foreground/70 sm:text-base sm:leading-7">{body}</p>
			{#if children}
				<div class="mt-7 flex flex-wrap items-center gap-3">
					{@render children()}
				</div>
			{/if}
		</div>

		{#if fan.length > 0}
			<!-- A fan of the library's own posters, as the reading hero shows a cover. -->
			<div class="relative mx-auto hidden h-72 w-[24rem] md:block lg:h-80 lg:w-[28rem]" aria-hidden="true">
				{#each fan as item, i (item.id)}
					{@const centre = i === 1 || fan.length === 1}
					<div
						class={[
							'absolute top-1/2 aspect-[2/3] overflow-hidden rounded-xl shadow-[0_30px_70px_rgba(0,0,0,0.55)] ring-1 ring-white/10',
							centre
								? 'left-1/2 z-10 w-40 -translate-x-1/2 -translate-y-1/2 lg:w-44'
								: i === 0
									? 'left-6 w-32 -translate-y-[46%] -rotate-[8deg] opacity-80 lg:w-36'
									: 'right-6 w-32 -translate-y-[46%] rotate-[8deg] opacity-80 lg:w-36'
						].join(' ')}
					>
						<Poster {item} enrichment={enrichmentCache.get(item.id) ?? null} frame="portrait" class="size-full" />
					</div>
				{/each}
			</div>
		{/if}
	</div>

	{#if steps.length > 0}
		<ol class="mt-10 grid gap-x-10 gap-y-5 border-t border-hairline pt-6 sm:grid-cols-3 md:mt-12">
			{#each steps as step, i (step.label)}
				<li class="min-w-0">
					<p class="font-mono text-[10px] uppercase tracking-[0.2em] text-muted">
						<span class="text-accent">{String(i + 1).padStart(2, '0')}</span>
						<span class="mx-1 opacity-50" aria-hidden="true">·</span>{step.label}
					</p>
					<p class="mt-2 text-sm leading-6 text-foreground/75">{step.text}</p>
				</li>
			{/each}
		</ol>
	{/if}
</section>
