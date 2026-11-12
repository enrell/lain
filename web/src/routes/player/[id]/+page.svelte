<script lang="ts">
	import { formatLength } from '$lib/utilities/format';
	import { onDestroy, onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Check from '@lucide/svelte/icons/check';
	import MonitorPlay from '@lucide/svelte/icons/monitor-play';
	import type { CatalogItem, PlaybackPlan, Progress } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Player from '$lib/components/player/Player.svelte';
	import ExternalPlayers from '$lib/components/player/ExternalPlayers.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { episodeLabel, nextEpisode } from '$lib/utilities/grouping';
	import { prefs } from '$lib/auth/storage';
	import { itemCache } from '$lib/stores/media-cache.svelte';
	import { loadPreferredPlayer, playbackHref, type PreferredPlayer } from '$lib/player/external-player';
	let preferredPlayer = $state<PreferredPlayer>('browser');
	let origin = $state('');
	onMount(() => {
		origin = window.location.origin;
		if (session.user) preferredPlayer = loadPreferredPlayer(session.user.id);
	});
	function playHref(itemId: string): string { return playbackHref(origin, itemId, preferredPlayer); }

	const id = $derived(page.params.id ?? '');

	let item = $state<CatalogItem | null>(null);
	let plan = $state<PlaybackPlan | null>(null);
	let progress = $state<Progress | null>(null);
	// What the title is about, for the column under an upright phone's player.
	let about = $state<{ synopsis: string; genres: string[] } | null>(null);
	$effect(() => {
		const current = item?.id;
		about = null;
		if (!current) return;
		void api.enrich
			.get(current)
			.then((e) => {
				if (item?.id === current && e) about = { synopsis: e.synopsis ?? '', genres: e.genres ?? [] };
			})
			.catch(() => {});
	});
	// The title's other files, listed in the sidebar so the viewer can
	// switch episodes without going back to the title page.
	let episodes = $state<CatalogItem[]>([]);
	let progressMap = $state<Map<string, Progress>>(new Map());
	let durations = $state<Map<string, number>>(new Map());
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);
	const AUTOPLAY_SECONDS = 8;
	let autoplayNext = $state(prefs.getBool('player.autoplay_next', true));
	// Seconds left before the next episode starts; null while nothing is queued.
	let countdown = $state<number | null>(null);
	let upNext = $state<CatalogItem | null>(null);
	let timer: ReturnType<typeof setInterval> | undefined;

	function cancelUpNext(): void {
		clearInterval(timer);
		timer = undefined;
		countdown = null;
		upNext = null;
	}

	function playNext(target: CatalogItem): void {
		cancelUpNext();
		void goto(playHref(target.id));
	}

	function onFinished(): void {
		if (!item || !autoplayNext) return;
		const target = nextEpisode(episodes, item.id);
		if (!target) return;
		upNext = target;
		countdown = AUTOPLAY_SECONDS;
		timer = setInterval(() => {
			if (countdown === null || !upNext) return;
			if (countdown <= 1) playNext(upNext);
			else countdown -= 1;
		}, 1000);
	}

	function toggleAutoplay(): void {
		autoplayNext = !autoplayNext;
		prefs.set('player.autoplay_next', String(autoplayNext));
		if (!autoplayNext) cancelUpNext();
	}

	onDestroy(cancelUpNext);

	const detailHref = $derived(item ? `/item/${item.id}` : '/library');

	async function load(): Promise<void> {
		loading = true;
		error = null;
		notFound = false;
		try {
			const [it, pl, prog, eps, allProgress] = await Promise.all([
				api.catalog.get(id),
				api.playback.plan(id),
				api.progress.get(id),
				api.catalog.episodes(id),
				api.me.continueWatching()
			]);
			item = it;
			itemCache.set(it.id, it);
			plan = pl;
			progress = prog;
			episodes = eps.items;
			progressMap = new Map(allProgress.map((p) => [p.item_id, p]));
			// How long each one is, for the list beside the picture.
			void api.playback.durations(eps.items.map((e) => e.id)).then((d) => (durations = d));
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'not-found') notFound = true;
			else error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	/*
	 * SvelteKit reuses this component when only the [id] param changes,
	 * which is exactly what the sidebar does. onMount would never run
	 * again and the previous episode would stay on screen, so the load
	 * follows the param instead. The effect reads nothing but `id`:
	 * everything load() touches is a write, so it cannot re-trigger
	 * itself.
	 */
	$effect(() => {
		void id;
		cancelUpNext();
		void load();
	});

	function stillUrl(episodeId: string): string {
		return api.thumbnail.url(episodeId, session.token, { width: 320 });
	}

	function progressLine(episode: CatalogItem): string {
		if (episode.missing) return 'Missing from disk';
		const p = progressMap.get(episode.id);
		// Its length, then its state, as the title page's cards say it.
		const length = formatLength(durations.get(episode.id));
		const with_ = (state: string): string => (length ? `${length} · ${state}` : state);
		if (!p) return length || 'Not watched';
		if (p.completed) return with_('Watched');
		if (p.duration_sec > 0 && p.position_sec > 0)
			// What is left, as every other place says it (not a percentage here and a time there).
			return `${formatLength(Math.max(1, p.duration_sec - p.position_sec))} left`;
		return length || 'Not watched';
	}
</script>

<svelte:head><title>{item?.title ? `Playing ${item.title} — Lain` : 'Player — Lain'}</title></svelte:head>

{#if loading && !item}
	<div class="flex h-dvh items-center justify-center bg-background">
		<Spinner class="size-8 text-muted" label="Preparing playback" />
	</div>
{:else if notFound}
	<div class="flex h-dvh flex-col items-center justify-center gap-4 bg-background px-6 text-center">
		<p class="text-sm text-muted">That item no longer exists in the catalog.</p>
		<Button variant="secondary" onclick={() => void goto('/library')}>
			<ArrowLeft class="size-4" /> Back to library
		</Button>
	</div>
{:else if error}
	<div class="flex h-dvh items-center justify-center bg-background px-6">
		<ErrorState message={error} retry={() => void load()} />
	</div>
{:else if item && plan}
	<div class="flex h-dvh flex-col bg-black lg:flex-row">
		<!-- A phone held upright: a square stage at the top, the picture in its
		     middle and the controls in the bands above and below it (on a 16:9
		     frame that small they would cover the picture), and what comes next
		     fills the rest. On its side, or full screen, it is all picture. -->
		<div class="relative min-h-0 min-w-0 flex-1 bg-black max-md:portrait:aspect-square max-md:portrait:flex-none">
			{#if plan.available}
				<!-- A new episode is a new session: keying on the item rebuilds
				     the player (video element, transcode session, clock) instead
				     of leaving the previous episode's state behind. -->
				{#key item.id}
					<Player {item} {plan} initialProgress={progress} {onFinished} />
				{/key}
				{#if upNext && countdown !== null}
					<div
						class="absolute bottom-24 right-5 z-30 w-72 border border-line/40 bg-black/85 p-4 backdrop-blur"
						role="status"
					>
						<p class="font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
							Up next · {countdown}s
						</p>
						<p class="mt-1 truncate text-sm font-semibold text-foreground">
							{episodeLabel(upNext)}
						</p>
						<div class="mt-3 flex gap-2">
							<Button variant="secondary" onclick={() => upNext && playNext(upNext)}>Play now</Button>
							<Button variant="ghost" onclick={cancelUpNext}>Cancel</Button>
						</div>
					</div>
				{/if}
			{:else}
				<div class="relative flex h-full items-center justify-center overflow-hidden bg-background px-6">
					<div class="lattice absolute inset-0 opacity-30"></div>
					<div class="relative max-w-lg text-center">
						<MonitorPlay class="mx-auto size-10 text-muted" />
						<h1 class="mt-5 text-xl font-semibold text-foreground">{item.title}</h1>
						<p class="mt-3 text-sm leading-relaxed text-muted">
							{plan.reason ?? 'No playback plan is available for this file in the browser.'}
						</p>
						{#if item.missing}
							<p class="mt-3 text-xs leading-relaxed text-muted/80">
								The entry stays in the catalog so progress and metadata are preserved. If the file
								comes back, it becomes playable again on its own.
							</p>
						{:else}
							<p class="mt-3 text-xs leading-relaxed text-muted/80">
								The server only sends bytes it can actually serve: the transcoder is unavailable, so
								there is no browser-playable version of this container. The desktop/CLI client plays
								it directly.
							</p>
						{/if}
						<div class="mt-7 flex flex-wrap items-center justify-center gap-2">
							<Button variant="secondary" onclick={() => void goto(detailHref)}>
								<ArrowLeft class="size-4" /> Back to details
							</Button>
							<Button variant="ghost" onclick={() => void goto('/library')}>Browse library</Button>
						</div>
						{#if !item.missing}<div class="mt-5"><ExternalPlayers itemId={item.id} /></div>{/if}
					</div>
				</div>
			{/if}
		</div>

		{#if episodes.length > 1}
			<aside
				class="hidden w-80 shrink-0 flex-col border-l border-line/40 bg-surface/60 lg:flex max-md:portrait:flex max-md:portrait:min-h-0 max-md:portrait:w-full max-md:portrait:flex-1 max-md:portrait:border-l-0 max-md:portrait:bg-background"
				aria-label="Episodes"
			>
				<div class="border-b border-line/40 px-4 py-3">
					<p class="truncate text-sm font-semibold text-foreground">{item.title}</p>
					<p class="mt-0.5 text-xs text-muted">{episodes.length} episodes</p>
					<button
						type="button"
						class="mt-2 inline-flex min-h-8 items-center gap-2 text-xs text-foreground/80 hover:text-foreground"
						role="switch"
						aria-checked={autoplayNext}
						onclick={toggleAutoplay}
					>
						<!-- A switch that looks like one: as small caps it read as a fact, not a choice. -->
						<!-- Drawn as the settings switch: accent track and accent-ink knob when on, a small dim knob when off. -->
						<span class={['relative h-4 w-7 rounded-full transition-colors', autoplayNext ? 'bg-accent' : 'bg-surface-active'].join(' ')}>
							<span class={['absolute rounded-full transition-all', autoplayNext ? 'left-3.5 top-0.5 size-3 bg-accent-fg' : 'left-[3px] top-[3px] size-2.5 bg-muted/60'].join(' ')}></span>
						</span>
						Autoplay next
					</button>
				</div>
				<ul class="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
					{#each episodes as episode (episode.id)}
						{@const current = episode.id === item.id}
						{@const p = progressMap.get(episode.id)}
						{@const begun = !!p && !p.completed && p.duration_sec > 0 && p.position_sec > 0}
						<li>
							<a
								href={episode.missing ? undefined : playHref(episode.id)}
								aria-disabled={episode.missing ? 'true' : undefined}
								aria-current={current ? 'true' : undefined}
								aria-label={`${episodeLabel(episode)}${current ? ', playing' : ''}`}
								class={[
									'flex items-center gap-3 p-2 transition-colors',
									current ? 'bg-foreground/5' : episode.missing ? 'opacity-50' : 'hover:bg-foreground/8'
								].join(' ')}
							>
								<span class="relative block h-12 w-20 shrink-0 overflow-hidden border border-line/40 bg-surface">
									<img
										src={stillUrl(episode.id)}
										alt=""
										aria-hidden="true"
										loading="lazy"
										decoding="async"
										class="size-full object-cover"
									/>
									<!-- The marks of the title page's episode cards: a check when watched, a strip
									     for the one under way; the list said both in words only. -->
									{#if p?.completed}
										<span class="absolute left-1 top-1 flex size-4 items-center justify-center rounded-full bg-black/70 text-white" aria-hidden="true">
											<Check class="size-2.5" />
										</span>
									{/if}
									{#if begun}
										<span class="absolute inset-x-0 bottom-0 h-1 bg-black/55" aria-hidden="true">
											<span class="block h-full bg-accent" style:width={`${Math.min(100, (p!.position_sec / p!.duration_sec) * 100)}%`}></span>
										</span>
									{/if}
								</span>
								<span class="min-w-0 flex-1">
									<span
										class={[
											'block text-xs font-semibold',
											current ? 'text-accent' : 'text-muted'
										].join(' ')}
									>
										{episodeLabel(episode)}
									</span>
									<!-- What is left of the one under way in the accent, as the app's episode rows. -->
									<span class={['mt-0.5 block truncate text-xs', begun && !episode.missing ? 'text-accent' : 'text-muted'].join(' ')}>{progressLine(episode)}</span>
								</span>
							</a>
						</li>
					{/each}
				</ul>
			</aside>
		{:else}
			<div class="hidden min-h-0 flex-1 overflow-y-auto bg-background px-5 py-5 max-md:portrait:block">
				<h1 class="text-xl font-semibold tracking-[-0.02em] text-foreground">{item.title}</h1>
				<p class="mt-1.5 text-sm text-muted">
					{[item.year > 0 ? String(item.year) : '', formatLength(plan?.duration_sec), ...(about?.genres ?? []).slice(0, 2)].filter(Boolean).join('  ·  ')}
				</p>
				{#if about?.synopsis}<p class="mt-4 text-sm leading-6 text-foreground/75">{about.synopsis}</p>{/if}
				<a href={`/item/${item.id}`} class="mt-5 inline-flex min-h-11 items-center gap-2 rounded-md border border-line px-4 text-sm font-medium text-foreground hover:bg-surface-hover">Details</a>
			</div>
		{/if}
	</div>
{/if}
