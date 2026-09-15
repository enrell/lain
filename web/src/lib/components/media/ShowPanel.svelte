<script lang="ts">
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Play from '@lucide/svelte/icons/play';
	import type { Library, Progress } from '$lib/api/types';
	import type { SeriesGroup } from '$lib/utilities/grouping';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import Poster from '$lib/components/media/Poster.svelte';

	/**
	 * Plex-style show page: a full-bleed backdrop with the title block,
	 * then a poster rail (resume action plus an information card)
	 * beside an episode card grid. Seasons filter; movies never reach
	 * here — single titles link straight to their item page.
	 */
	let {
		group,
		progressMap = null,
		libraries = [],
		onback
	}: {
		group: SeriesGroup;
		progressMap?: Map<string, Progress> | null;
		libraries?: Library[];
		onback: () => void;
	} = $props();

	const seasons = $derived([...new Set(group.items.map((i) => i.season).filter((s) => s > 0))].sort((a, b) => a - b));
	let season = $state<number | 'all'>('all');

	const visible = $derived(
		season === 'all' ? group.items : group.items.filter((i) => i.season === season)
	);

	const first = $derived(group.items[0]);
	const artItem = $derived(group.items.find((i) => i.episode > 0) ?? first);
	const enrichments = $derived(group.items.map((i) => enrichmentCache.get(i.id)));
	const synopsis = $derived(enrichments.map((e) => e?.synopsis).find((s) => s && s.length > 0) ?? '');
	const genres = $derived(enrichments.map((e) => e?.genres ?? []).find((g) => g.length > 0)?.slice(0, 3) ?? []);
	const backdrop = $derived(enrichments.map((e) => e?.cover).find((c) => c) ?? '');
	const posterEnrichment = $derived(enrichmentCache.get(artItem.id) ?? null);

	const meta = $derived(
		[
			group.year > 0 ? String(group.year) : '',
			'Series',
			`${group.count} ${group.count === 1 ? 'file' : 'files'}`
		]
			.filter(Boolean)
			.join('  ·  ')
	);

	const libraryNames = $derived(
		libraries
			.filter((lib) => group.libraryIds.includes(lib.id))
			.map((lib) => lib.name)
			.filter((name, index, all) => all.indexOf(name) === index)
	);

	const watched = $derived(
		group.items.filter((i) => progressMap?.get(i.id)?.completed).length
	);

	// Resume at the first started-but-unfinished episode; otherwise play
	// the show from the top like a fresh watch.
	const resumeTarget = $derived(
		group.items.find((i) => {
			const p = progressMap?.get(i.id);
			return p && !p.completed && p.position_sec >= 5;
		}) ?? first
	);
	const resumeProgress = $derived(progressMap?.get(resumeTarget.id) ?? null);
	const resumeLabel = $derived(
		resumeProgress && !resumeProgress.completed && resumeProgress.position_sec >= 5
			? `Resume · ${episodeLabel(resumeTarget)}`
			: 'Play'
	);
	const watchedLine = $derived(
		resumeProgress && !resumeProgress.completed && resumeProgress.duration_sec > 0
			? `${Math.round((resumeProgress.position_sec / resumeProgress.duration_sec) * 100)}% watched`
			: watched > 0
				? `${watched} of ${group.count} watched`
				: 'Not started'
	);

	function episodeLabel(item: { season: number; episode: number; title: string }): string {
		if (item.season > 0 && item.episode > 0)
			return `S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`;
		if (item.episode > 0) return `Episode ${item.episode}`;
		return item.title;
	}

	function stillUrl(id: string): string {
		return api.thumbnail.url(id, session.token, { width: 480 });
	}

	function formatSize(bytes: number): string {
		if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(2)} GB`;
		return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
	}

	function cardProgress(id: string): number {
		const p = progressMap?.get(id);
		if (!p || p.completed || p.duration_sec <= 0) return 0;
		return Math.min(100, Math.round((p.position_sec / p.duration_sec) * 100));
	}
</script>

<article class="space-y-8">
	<div class="relative overflow-hidden rounded-2xl border border-line/70 bg-surface/30">
		{#if backdrop}
			<img src={backdrop} alt="" aria-hidden="true" class="absolute inset-0 size-full object-cover" referrerpolicy="no-referrer" />
			<div class="absolute inset-0 bg-gradient-to-t from-background via-background/60 to-background/10"></div>
		{/if}
		<button
			type="button"
			onclick={onback}
			class="absolute left-4 top-4 inline-flex items-center gap-1.5 rounded-full border border-white/15 bg-black/45 px-3 py-1.5 text-xs font-semibold text-white backdrop-blur transition-colors hover:bg-black/65"
		>
			<ArrowLeft class="size-3.5" aria-hidden="true" /> Library
		</button>
		<div class="relative px-5 pb-7 pt-28 sm:px-8 sm:pt-40">
			<p class="font-mono text-[10px] uppercase tracking-[0.18em] text-muted">{meta}</p>
			<h1 class="mt-2 max-w-4xl text-3xl font-bold tracking-[-0.03em] text-white sm:text-5xl">{group.title}</h1>
			{#if genres.length > 0}
				<ul class="mt-4 flex flex-wrap gap-2" aria-label="Genres">
					{#each genres as genre (genre)}
						<li class="rounded-full border border-white/15 bg-white/5 px-3 py-1 text-xs font-semibold text-white/90">{genre}</li>
					{/each}
				</ul>
			{/if}
			{#if synopsis}
				<p class="mt-4 line-clamp-3 max-w-3xl text-sm leading-6 text-white/75">{synopsis}</p>
			{/if}
		</div>
	</div>

	<div class="grid items-start gap-8 lg:grid-cols-[280px_minmax(0,1fr)]">
		<div class="space-y-5 lg:sticky lg:top-24">
			<div class="aspect-[2/3] w-44 overflow-hidden rounded-xl bg-surface shadow-2xl ring-1 ring-white/10 lg:w-full">
				<Poster item={artItem} enrichment={posterEnrichment} priority class="size-full" />
			</div>
			<div>
				<LinkButton href={`/player/${resumeTarget.id}`} size="lg" class="w-full justify-center">
					<Play class="size-4 fill-current" aria-hidden="true" /> {resumeLabel}
				</LinkButton>
				<p class="mt-2 text-center text-xs text-muted" aria-live="polite">{watchedLine}</p>
			</div>
			<div class="rounded-xl border border-line/70 bg-surface/30 p-4">
				<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-muted">Information</h2>
				<dl class="mt-2 text-sm">
					<div class="flex items-center justify-between gap-3 border-t border-line/60 py-2">
						<dt class="text-xs uppercase tracking-wider text-muted">Format</dt>
						<dd class="font-semibold text-foreground">Series</dd>
					</div>
					{#if group.year > 0}
						<div class="flex items-center justify-between gap-3 border-t border-line/60 py-2">
							<dt class="text-xs uppercase tracking-wider text-muted">Year</dt>
							<dd class="font-semibold text-foreground">{group.year}</dd>
						</div>
					{/if}
					{#if seasons.length > 0}
						<div class="flex items-center justify-between gap-3 border-t border-line/60 py-2">
							<dt class="text-xs uppercase tracking-wider text-muted">Seasons</dt>
							<dd class="font-semibold text-foreground">{seasons.length}</dd>
						</div>
					{/if}
					<div class="flex items-center justify-between gap-3 border-t border-line/60 py-2">
						<dt class="text-xs uppercase tracking-wider text-muted">Episodes</dt>
						<dd class="font-semibold text-foreground">{group.count}</dd>
					</div>
					{#if libraryNames.length > 0}
						<div class="flex items-center justify-between gap-3 border-t border-line/60 py-2">
							<dt class="text-xs uppercase tracking-wider text-muted">Library</dt>
							<dd class="truncate font-semibold text-foreground">{libraryNames.join(' · ')}</dd>
						</div>
					{/if}
					<div class="flex items-center justify-between gap-3 border-y border-line/60 py-2">
						<dt class="text-xs uppercase tracking-wider text-muted">Watched</dt>
						<dd class="font-semibold text-foreground">{watched} of {group.count}</dd>
					</div>
				</dl>
			</div>
		</div>

		<div class="min-w-0">
			{#if seasons.length > 1}
				<div class="mb-4 flex flex-wrap gap-2" role="group" aria-label="Seasons">
					<button
						type="button"
						onclick={() => (season = 'all')}
						aria-pressed={season === 'all'}
						class={[
							'rounded-full px-3.5 py-1.5 text-xs font-semibold transition-colors',
							season === 'all' ? 'bg-foreground text-background' : 'bg-surface text-muted hover:text-foreground'
						].join(' ')}
					>
						All
					</button>
					{#each seasons as s (s)}
						<button
							type="button"
							onclick={() => (season = s)}
							aria-pressed={season === s}
							class={[
								'rounded-full px-3.5 py-1.5 text-xs font-semibold transition-colors',
								season === s ? 'bg-foreground text-background' : 'bg-surface text-muted hover:text-foreground'
							].join(' ')}
						>
							Season {s}
						</button>
					{/each}
				</div>
			{/if}

			<ul class="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3">
				{#each visible as item (item.id)}
					{@const progress = progressMap?.get(item.id) ?? null}
					{@const ratio = cardProgress(item.id)}
					<li>
						<a
							href={`/item/${item.id}`}
							class="group block overflow-hidden rounded-xl border border-line/60 bg-surface/30 transition duration-300 hover:-translate-y-1 hover:border-white/15"
							aria-label={`${episodeLabel(item)}${item.title !== group.title ? `, ${item.title}` : ''}`}
						>
							<div class="relative aspect-video overflow-hidden bg-surface">
								<img
									src={stillUrl(item.id)}
									alt=""
									aria-hidden="true"
									loading="lazy"
									decoding="async"
									class="size-full object-cover transition-transform duration-500 ease-out group-hover:scale-[1.04]"
								/>
								<span
									class="absolute right-2 top-2 flex size-8 items-center justify-center rounded-full bg-black/65 text-white opacity-0 backdrop-blur transition-opacity duration-200 group-hover:opacity-100"
								>
									<Play class="size-3.5 translate-x-px" aria-hidden="true" />
								</span>
								{#if ratio > 0}
									<div class="absolute inset-x-0 bottom-0 h-1 bg-black/50">
										<div class="h-full bg-accent" style={`width: ${ratio}%`}></div>
									</div>
								{/if}
							</div>
							<div class="p-3">
								<p class="font-mono text-[10px] uppercase tracking-[0.14em] text-accent">{episodeLabel(item)}</p>
								{#if item.title !== group.title}
									<p class="mt-1 truncate text-sm font-semibold text-foreground">{item.title}</p>
								{/if}
								<p class="mt-1 truncate text-xs text-muted">
									{#if progress && !progress.completed && progress.duration_sec > 0}
										Resume at {Math.min(100, Math.round((progress.position_sec / progress.duration_sec) * 100))}%
									{:else if progress?.completed}
										Watched
									{:else if item.size > 0}
										{formatSize(item.size)}
									{:else}
										Not watched
									{/if}
								</p>
							</div>
						</a>
					</li>
				{/each}
			</ul>
		</div>
	</div>
</article>
