<script lang="ts">
	import { formatLength } from '$lib/utilities/format';
	import { onDestroy, onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Check from '@lucide/svelte/icons/check';
	import MonitorPlay from '@lucide/svelte/icons/monitor-play';
	import PanelRightClose from '@lucide/svelte/icons/panel-right-close';
	import type { CatalogItem, PlaybackPlan, Progress } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Player from '$lib/components/player/Player.svelte';
	import ExternalPlayers from '$lib/components/player/ExternalPlayers.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { scrollEdges } from '$lib/utilities/scroll-edges';
	import { episodeLabel, nextEpisode } from '$lib/utilities/grouping';
	import { prefs } from '$lib/auth/storage';
	import { itemCache } from '$lib/stores/media-cache.svelte';
	import { wideStill } from '$lib/stores/frames';
	import { loadPreferredPlayer, playbackHref, type PreferredPlayer } from '$lib/player/external-player';
	import { t } from '$lib/i18n';
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
	/*
	 * Where the episode list goes. Beside the picture on a wide screen (lg and
	 * up), shown or put away by the viewer, a choice this browser keeps. Under
	 * the picture on an upright phone, always there. In between (a tablet, a
	 * phone on its side) there is no room beside the picture, so the same
	 * button and E open it as a drawer over the picture's edge, closed again
	 * by Escape, the button or a click outside; it starts closed, since it
	 * covers what is playing.
	 */
	let wide = $state(false);
	let upright = $state(false);
	onMount(() => {
		const wideQuery = window.matchMedia('(min-width: 1024px)');
		const uprightQuery = window.matchMedia('(max-width: 767px) and (orientation: portrait)');
		const sync = (): void => {
			wide = wideQuery.matches;
			upright = uprightQuery.matches;
		};
		sync();
		wideQuery.addEventListener('change', sync);
		uprightQuery.addEventListener('change', sync);
		return () => {
			wideQuery.removeEventListener('change', sync);
			uprightQuery.removeEventListener('change', sync);
		};
	});
	const overlay = $derived(!wide && !upright);
	let listShown = $state(prefs.getBool('player.episode_list', true));
	let drawerOpen = $state(false);
	let listEl = $state<HTMLElement | null>(null);
	const episodesToggle = {
		get shown() {
			return overlay ? drawerOpen : listShown;
		},
		get overlay() {
			return overlay;
		},
		toggle: () => {
			if (overlay) {
				drawerOpen = !drawerOpen;
				// A drawer is entered: the keyboard lands on the episode playing.
				// Closed, the keyboard goes back to the button that opened it.
				if (drawerOpen)
					void tick().then(() => (listEl?.querySelector<HTMLElement>('[aria-current="true"]') ?? listEl?.querySelector<HTMLElement>('a[href]'))?.focus());
				else document.querySelector<HTMLElement>('[data-episodes-toggle] button')?.focus();
				return;
			}
			// Put away from its own Hide button, the list would take the focus
			// with it to the page body: it goes to the bar's button instead.
			const fromList = !!listEl?.contains(document.activeElement);
			listShown = !listShown;
			prefs.set('player.episode_list', String(listShown));
			if (!listShown && fromList) document.querySelector<HTMLElement>('[data-episodes-toggle] button')?.focus();
		}
	};
	/*
	 * The drawer covers the picture behind a backdrop, so it holds the
	 * keyboard as a dialog does: Tab and Shift+Tab go round its own controls
	 * instead of walking into the player's bar under the backdrop.
	 */
	$effect(() => {
		const drawer = listEl;
		if (!drawer || !overlay || !drawerOpen) return;
		const onTab = (event: KeyboardEvent): void => {
			if (event.key !== 'Tab') return;
			const stops = [...drawer.querySelectorAll<HTMLElement>('button, a[href]')].filter((el) => el.getClientRects().length > 0);
			const first = stops[0];
			const last = stops.at(-1);
			if (!first || !last) return;
			if (event.shiftKey && document.activeElement === first) {
				event.preventDefault();
				last.focus();
			} else if (!event.shiftKey && document.activeElement === last) {
				event.preventDefault();
				first.focus();
			}
		};
		drawer.addEventListener('keydown', onTab);
		return () => drawer.removeEventListener('keydown', onTab);
	});
	/*
	 * The episode playing is in view in the list: playing episode 12 of 12,
	 * the list opened on 1 to 11 with 12 cut off at the bottom. Scrolled only
	 * when it is not already in view, and only the list, never the page; again
	 * when the list comes back, since a hidden list forgets where it was.
	 */
	let listScroller = $state<HTMLElement | null>(null);
	$effect(() => {
		const scroller = listScroller;
		const current = item?.id;
		void episodesToggle.shown;
		if (!scroller || !current || episodes.length === 0) return;
		void tick().then(() => {
			const row = scroller.querySelector<HTMLElement>('[aria-current="true"]')?.closest('li');
			if (!row) return;
			const top = row.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop;
			const bottom = top + row.offsetHeight;
			// In view means clear of the ends' fade (the list's scroll padding).
			const pad = parseFloat(getComputedStyle(scroller).scrollPaddingTop) || 0;
			if (top >= scroller.scrollTop + pad && bottom <= scroller.scrollTop + scroller.clientHeight - pad) return;
			scroller.scrollTop = Math.max(0, top - (scroller.clientHeight - row.offsetHeight) / 2);
		});
	});

	// Another episode, or a screen that changes where the list goes, closes the drawer.
	$effect(() => {
		void id;
		void overlay;
		drawerOpen = false;
	});
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

	// The episode after this one, for the bar's Next button and Shift+N.
	const following = $derived(item ? nextEpisode(episodes, item.id) : null);
	const next = $derived(following ? { label: episodeLabel(following), go: () => playNext(following) } : null);

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

	function progressLine(episode: CatalogItem): string {
		if (episode.missing) return t('player.list.missing');
		const p = progressMap.get(episode.id);
		// Its length, then its state, as the title page's cards say it.
		const length = formatLength(durations.get(episode.id));
		const with_ = (state: string): string => (length ? t('player.list.lengthAndState', { length, state }) : state);
		if (!p) return length || t('player.list.notWatched');
		if (p.completed) return with_(t('player.list.watched'));
		if (p.duration_sec > 0 && p.position_sec > 0)
			// What is left, as every other place says it (not a percentage here and a time there).
			return t('player.list.left', { time: formatLength(Math.max(1, p.duration_sec - p.position_sec)) });
		return length || t('player.list.notWatched');
	}
</script>

<svelte:head><title>{item?.title ? t('player.pageTitle', { title: item.title }) : t('player.pageTitleEmpty')}</title></svelte:head>

{#if loading && !item}
	<div class="flex h-dvh items-center justify-center bg-background">
		<Spinner class="size-8 text-muted" label={t('player.prepare.label')} />
	</div>
{:else if notFound}
	<div class="flex h-dvh flex-col items-center justify-center gap-4 bg-background px-6 text-center">
		<p class="text-sm text-muted">{t('player.notFound')}</p>
		<Button variant="secondary" onclick={() => void goto('/library')}>
			<ArrowLeft class="size-4" /> {t('player.backToLibrary')}
		</Button>
	</div>
{:else if error}
	<div class="flex h-dvh items-center justify-center bg-background px-6">
		<ErrorState message={error} retry={() => void load()} />
	</div>
{:else if item && plan}
	<div class="relative flex h-dvh flex-col bg-black lg:flex-row" data-immersive>
		<!-- A phone held upright: the stage at the top, as tall as its bands
		     (the top bar, the 16:9 picture, the deck: the player stacks them,
		     since on a frame that small the controls would cover the picture),
		     and what comes next fills the rest. On its side, or full screen, it
		     is all picture. A title that cannot play keeps a square for its note. -->
		<div class={['relative min-h-0 min-w-0 flex-1 bg-black max-md:portrait:flex-none', plan.available ? '' : 'max-md:portrait:aspect-square'].join(' ')}>
			{#if plan.available}
				<!-- A new episode is a new session: keying on the item rebuilds
				     the player (video element, transcode session, clock) instead
				     of leaving the previous episode's state behind. -->
				{#key item.id}
					<Player {item} {plan} initialProgress={progress} {onFinished} {next} episodes={episodes.length > 1 && !upright ? episodesToggle : null} />
				{/key}
				{#if upNext && countdown !== null}
					<div
						class="absolute bottom-24 right-5 z-30 w-72 border border-line/40 bg-black/85 p-4 backdrop-blur"
						role="status"
					>
						<p class="font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
							{t('player.upNext.kicker', { seconds: countdown })}
						</p>
						<p class="mt-1 truncate text-sm font-semibold text-foreground">
							{episodeLabel(upNext)}
						</p>
						<div class="mt-3 flex gap-2">
							<Button variant="secondary" onclick={() => upNext && playNext(upNext)}>{t('player.upNext.playNow')}</Button>
							<Button variant="ghost" onclick={cancelUpNext}>{t('common.cancel')}</Button>
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
							{plan.reason ?? t('player.unavailable.reason')}
						</p>
						{#if item.missing}
							<p class="mt-3 text-xs leading-relaxed text-muted/80">{t('player.unavailable.missing')}</p>
						{:else}
							<p class="mt-3 text-xs leading-relaxed text-muted/80">{t('player.unavailable.noTranscoder')}</p>
						{/if}
						<div class="mt-7 flex flex-wrap items-center justify-center gap-2">
							<Button variant="secondary" onclick={() => void goto(detailHref)}>
								<ArrowLeft class="size-4" /> {t('player.backToDetails')}
							</Button>
							<Button variant="ghost" onclick={() => void goto('/library')}>{t('player.browseLibrary')}</Button>
						</div>
						{#if !item.missing}<div class="mt-5"><ExternalPlayers itemId={item.id} /></div>{/if}
					</div>
				</div>
			{/if}
		</div>

		{#if episodes.length > 1}
			{#if overlay && drawerOpen}
				<!-- Outside the drawer: a click there puts it away (Escape and E do too). -->
				<button type="button" tabindex="-1" aria-hidden="true" class="absolute inset-0 z-30 cursor-default bg-black/40" onclick={episodesToggle.toggle}></button>
			{/if}
			<aside
				bind:this={listEl}
				class={[
					'w-80 shrink-0 flex-col border-l border-line/40',
					overlay && drawerOpen
						? 'absolute inset-y-0 right-0 z-40 flex max-w-[85vw] bg-surface/95 shadow-[-18px_0_45px_rgba(0,0,0,0.45)] backdrop-blur-xl'
						: `hidden bg-surface/60 ${listShown ? 'lg:flex' : ''}`,
					'max-md:portrait:flex max-md:portrait:min-h-0 max-md:portrait:w-full max-md:portrait:flex-1 max-md:portrait:border-l-0 max-md:portrait:bg-background'
				].join(' ')}
				aria-label={t('player.list.label')}
			>
				<div class="relative border-b border-line/40 px-4 py-3">
					<!-- Put away from the list itself, with the key that does it: the bar's
					     button alone was easy to miss. Not on an upright phone, where the
					     list is the page under the picture. -->
					{#if !upright}
						<button
							type="button"
							class="absolute right-2 top-2 inline-flex min-h-8 items-center gap-1.5 rounded-md px-2 text-xs text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
							aria-label={overlay ? t('player.list.close') : t('player.episodesHide')}
							title={overlay ? t('player.list.close') : t('player.episodesHide')}
							onclick={episodesToggle.toggle}
						>
							<PanelRightClose class="size-4" aria-hidden="true" />
							<span>{t('player.list.hide')}</span>
							<kbd>E</kbd>
						</button>
					{/if}
					<p class="truncate pr-24 text-sm font-semibold text-foreground">{item.title}</p>
					<p class="mt-0.5 text-xs text-muted">{t('player.list.count', { count: episodes.length })}</p>
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
						{t('player.list.autoplay')}
					</button>
				</div>
				<!-- Fades where more episodes are hidden, above or below: at rest the
				     last one in view was cut by the window's edge with nothing to say
				     the list goes on. The scroll padding is the fade's 3rem, so an
				     episode Tab moves to stops clear of it instead of under it. -->
				<ul bind:this={listScroller} class="edge-fade-y min-h-0 flex-1 scroll-py-12 space-y-1 overflow-y-auto p-2" {@attach scrollEdges(undefined, '3rem', 'y')}>
					{#each episodes as episode (episode.id)}
						{@const current = episode.id === item.id}
						{@const p = progressMap.get(episode.id)}
						{@const begun = !!p && !p.completed && p.duration_sec > 0 && p.position_sec > 0}
						<li>
							<a
								href={episode.missing ? undefined : playHref(episode.id)}
								aria-disabled={episode.missing ? 'true' : undefined}
								aria-current={current ? 'true' : undefined}
								aria-label={current ? t('player.list.playing', { episode: episodeLabel(episode) }) : episodeLabel(episode)}
								class={[
									'flex items-center gap-3 p-2 transition-colors',
									current ? 'bg-foreground/5' : episode.missing ? 'opacity-50' : 'hover:bg-foreground/8'
								].join(' ')}
							>
								<span class="relative block h-12 w-20 shrink-0 overflow-hidden border border-line/40 bg-surface">
									<img
										src={wideStill(episode, 320)}
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
										<!-- On a dark track of its own, as the cards' bar: on a pale still the fill vanished. -->
										<span class="absolute inset-x-0 bottom-0 h-1 bg-black/75" aria-hidden="true">
											<span class="block h-full bg-accent" style:width={`${Math.max(4, Math.min(100, (p!.position_sec / p!.duration_sec) * 100))}%`}></span>
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
				<a href={`/item/${item.id}`} class="mt-5 inline-flex min-h-11 items-center gap-2 rounded-md border border-line px-4 text-sm font-medium text-foreground hover:bg-surface-hover">{t('player.details')}</a>
			</div>
		{/if}
	</div>
{/if}
