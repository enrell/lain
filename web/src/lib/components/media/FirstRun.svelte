<script lang="ts">
	/*
	 * What a page is for, before it has anything of yours to show: an
	 * accent eyebrow, a large tight line and one or two of plain text, the
	 * first step as the main action, and art from this server's own
	 * library, as Home's hero opens. Not a box: the art's colour washes the
	 * page behind the words and fades out at every edge.
	 *
	 * Each page draws what it is about with the library's own titles, so
	 * Tracking, Social and Acquire do not open on the same picture:
	 * - `list`: a list kept elsewhere, titles with how far along each is;
	 * - `people`: a title among three people, joined as the app's mark joins
	 *   its nodes;
	 * - `queue`: releases on their way into the library;
	 * - `fan`: three posters fanned out.
	 * Beside the words from a tablet's width; a phone has the words alone.
	 *
	 * Steps, when given, sit under a hairline as short mono-labelled notes,
	 * the way a rail's heading sits over its cards.
	 */
	import { onMount, type Snippet } from 'svelte';
	import { api } from '$lib/api';
	import type { CatalogItem } from '$lib/api/types';
	import Mascot from '$lib/components/profile/Mascot.svelte';
	import { t } from '$lib/i18n';
	import { cardPicture } from '$lib/stores/frames';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { guessShape, knownShape, measure, shapeOf } from '$lib/stores/shapes.svelte';
	import { artItem } from '$lib/utilities/browse';
	import { groupItems, type SeriesGroup } from '$lib/utilities/grouping';
	import Poster from './Poster.svelte';

	type Art = 'fan' | 'list' | 'people' | 'queue';

	let {
		eyebrow,
		title,
		body,
		steps = [],
		art = 'fan',
		children
	}: {
		eyebrow: string;
		title: string;
		body: string;
		/** Short notes under the actions: a mono label and one line each. */
		steps?: { label: string; text: string }[];
		/** What to draw beside the words from the library's titles, or nothing. */
		art?: Art | false;
		/** The actions: the first step first. */
		children?: Snippet;
	} = $props();

	const WANTED: Record<Art, number> = { fan: 3, list: 4, people: 1, queue: 3 };

	interface Pick {
		group: SeriesGroup;
		item: CatalogItem;
	}
	let picks = $state<Pick[]>([]);

	// The newest titles that stand upright, so the art reads as posters; a
	// wide still fills in only when there are too few.
	onMount(async () => {
		if (!art) return;
		const wanted = Math.max(3, WANTED[art]);
		try {
			const page = await api.catalog.list({ sort: 'recent', limit: 60 });
			const groups = groupItems(page.items).slice(0, 12);
			const items = groups.map(artItem);
			await ensureEnrichments(items.map((i) => i.id));
			// Pictures are loaded first only when what is known of them (D-282)
			// does not already name enough upright ones.
			const known = (i: CatalogItem) => knownShape(i, enrichmentCache.get(i.id));
			if (items.filter((i) => known(i) === 'portrait').length < wanted) {
				const unknown = items.filter((i) => known(i) === undefined);
				await measure(unknown.map((i) => ({ id: i.id, src: cardPicture(i, enrichmentCache.get(i.id)) })), 1500);
			}
			const all = groups.map((group, i) => ({ group, item: items[i] }));
			const upright = (p: Pick) => shapeOf(p.item.id, guessShape(p.item, enrichmentCache.get(p.item.id))) === 'portrait';
			picks = [...all.filter(upright), ...all.filter((p) => !upright(p))].slice(0, wanted);
		} catch {
			// Decoration: without it the words stand alone.
		}
	});

	const lead = $derived(picks[1] ?? picks[0]);
	const fan = $derived(picks.slice(0, 3));
	const nameOf = (p: Pick) => enrichmentCache.get(p.item.id)?.title || p.group.title;
	// A drawn row's second line: the year only. The newest files hold only part
	// of a long show, so a count of its episodes here would be short.
	const yearOf = (p: Pick) => (p.group.year > 0 ? String(p.group.year) : '');
	// How far along each row of the drawn list is: a picture of a list, not a record of one.
	const LIST_FILL = [1, 0.62, 0.3, 0];
	const QUEUE = [
		{ state: 'done', fill: 1 },
		{ state: 'downloading', fill: 0.58 },
		{ state: 'queued', fill: 0 }
	] as const;
	// Where the people stand around the title: the corners of the app's mark (SignalMark).
	const PEOPLE = [
		{ mascot: 'wired', at: 'left-0 top-6' },
		{ mascot: 'glyph', at: 'right-0 top-0' },
		{ mascot: 'neon', at: 'bottom-0 left-1/2 -translate-x-1/2' }
	];
</script>

{#snippet thumb(p: Pick, size: string)}
	<div class={['shrink-0 overflow-hidden rounded-md ring-1 ring-white/10', size].join(' ')}>
		<Poster item={p.item} enrichment={enrichmentCache.get(p.item.id) ?? null} frame="portrait" class="aspect-[2/3] size-full" />
	</div>
{/snippet}

<section class="relative isolate">
	{#if lead}
		<!-- The lead title's colours behind everything, faded out on every side. -->
		<div
			class="pointer-events-none absolute -top-16 -right-10 -z-10 h-[calc(100%+8rem)] w-[calc(100%+5rem)] overflow-hidden opacity-45 [mask-image:radial-gradient(closest-side,black_15%,transparent)] md:w-[75%]"
			aria-hidden="true"
		>
			<Poster item={lead.item} enrichment={enrichmentCache.get(lead.item.id) ?? null} class="size-full scale-110 blur-3xl" />
		</div>
	{/if}

	<!-- Words and art side by side from a tablet's width, the art smaller there
	     and the line a step smaller, so the words keep three or four a line. -->
	<div class="grid items-center gap-10 md:grid-cols-[minmax(0,1fr)_auto] lg:gap-16">
		<div class="min-w-0 py-2 md:py-8">
			<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent sm:text-[11px]">{eyebrow}</p>
			<h2 class="mt-4 max-w-[18ch] text-[2rem] font-semibold leading-[1.04] tracking-[-0.035em] text-balance text-foreground sm:text-5xl sm:leading-[1.02] md:text-4xl lg:text-5xl">
				{title}
			</h2>
			<p class="mt-4 max-w-xl text-sm leading-6 text-foreground/70 sm:text-base sm:leading-7">{body}</p>
			{#if children}
				<div class="mt-7 flex flex-wrap items-center gap-3">
					{@render children()}
				</div>
			{/if}
		</div>

		{#if picks.length > 0 && art}
			<div class="relative mx-auto hidden md:block" aria-hidden="true">
				{#if art === 'fan'}
					<!-- Three posters fanned out, the middle one in front. -->
					<div class="relative h-72 w-72 lg:h-80 lg:w-[28rem]">
						{#each fan as p, i (p.item.id)}
							{@const centre = i === 1 || fan.length === 1}
							<div
								class={[
									'absolute top-1/2 aspect-[2/3] overflow-hidden rounded-xl shadow-[0_30px_70px_rgba(0,0,0,0.55)] ring-1 ring-white/10',
									centre
										? 'left-1/2 z-10 w-36 -translate-x-1/2 -translate-y-1/2 lg:w-44'
										: i === 0
											? 'left-0 w-28 -translate-y-[46%] -rotate-[8deg] opacity-80 lg:left-6 lg:w-36'
											: 'right-0 w-28 -translate-y-[46%] rotate-[8deg] opacity-80 lg:right-6 lg:w-36'
								].join(' ')}
							>
								<Poster item={p.item} enrichment={enrichmentCache.get(p.item.id) ?? null} frame="portrait" class="size-full" />
							</div>
						{/each}
					</div>
				{:else if art === 'list'}
					<!-- A list kept elsewhere: a sheet of titles, how far along each one is,
					     over a second sheet behind it. -->
					<div class="relative w-64 lg:w-[22rem]">
						<div class="absolute inset-0 translate-x-3 translate-y-4 rotate-[4deg] rounded-2xl bg-surface/50 ring-1 ring-white/5"></div>
						<ol class="relative -rotate-[2deg] rounded-2xl bg-surface/85 px-4 py-2 shadow-[0_30px_70px_rgba(0,0,0,0.55)] ring-1 ring-white/10 backdrop-blur">
							{#each picks.slice(0, 4) as p, i (p.item.id)}
								<li class="flex items-center gap-3 border-hairline py-2.5 [&:not(:first-child)]:border-t">
									{@render thumb(p, 'w-9 lg:w-10')}
									<div class="min-w-0 flex-1">
										<p class="truncate text-sm font-semibold text-foreground">{nameOf(p)}</p>
										<p class="mt-0.5 truncate font-mono text-[10px] uppercase tracking-[0.14em] text-muted">{yearOf(p)}</p>
									</div>
									<div class="h-1 w-10 shrink-0 overflow-hidden rounded-full bg-line lg:w-14">
										<div class="h-full rounded-full bg-accent" style:width="{LIST_FILL[i] * 100}%"></div>
									</div>
								</li>
							{/each}
						</ol>
					</div>
				{:else if art === 'people'}
					<!-- A title among three people, joined as the app's mark joins its nodes. -->
					<div class="relative h-72 w-64 lg:h-80 lg:w-[22rem]">
						<svg class="absolute inset-0 size-full text-accent/35" viewBox="0 0 100 100" preserveAspectRatio="none" fill="none">
							<path d="M12 22 L88 14 L50 90 Z" stroke="currentColor" stroke-width="0.6" stroke-dasharray="2 2" vector-effect="non-scaling-stroke" />
						</svg>
						<div class="absolute left-1/2 top-1/2 w-32 -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-xl shadow-[0_30px_70px_rgba(0,0,0,0.55)] ring-1 ring-white/10 lg:w-40">
							<Poster item={picks[0].item} enrichment={enrichmentCache.get(picks[0].item.id) ?? null} frame="portrait" class="aspect-[2/3] size-full" />
						</div>
						{#each PEOPLE as person (person.mascot)}
							<div class={['absolute size-14 overflow-hidden rounded-full shadow-[0_12px_30px_rgba(0,0,0,0.5)] ring-4 ring-background lg:size-16', person.at].join(' ')}>
								<Mascot id={person.mascot} class="size-full" />
							</div>
						{/each}
					</div>
				{:else}
					<!-- Releases on their way into the library: one in, one coming, one waiting. -->
					<div class="relative w-64 lg:w-[22rem]">
						<ol class="relative rotate-[2deg] rounded-2xl bg-surface/85 px-4 py-2 shadow-[0_30px_70px_rgba(0,0,0,0.55)] ring-1 ring-white/10 backdrop-blur">
							{#each picks.slice(0, 3) as p, i (p.item.id)}
								{@const row = QUEUE[i]}
								<li class="flex items-center gap-3 border-hairline py-3 [&:not(:first-child)]:border-t">
									{@render thumb(p, ['w-9 lg:w-10', row.fill === 0 ? 'opacity-50' : ''].join(' '))}
									<div class="min-w-0 flex-1">
										<p class="truncate text-sm font-semibold text-foreground">{nameOf(p)}</p>
										<div class="mt-2 h-1 w-full overflow-hidden rounded-full bg-line">
											<div class="h-full rounded-full bg-accent" style:width="{row.fill * 100}%"></div>
										</div>
									</div>
									<span class={['w-24 shrink-0 text-right font-mono text-[10px] uppercase tracking-[0.14em]', row.state === 'done' ? 'text-accent' : 'text-muted'].join(' ')}>
										{t(`acquire.state.${row.state}`)}
									</span>
								</li>
							{/each}
						</ol>
					</div>
				{/if}
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
