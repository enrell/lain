<script lang="ts">
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import MonitorPlay from '@lucide/svelte/icons/monitor-play';
	import type { CatalogItem, PlaybackPlan, Progress } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Player from '$lib/components/player/Player.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { episodeLabel } from '$lib/utilities/grouping';
	import { itemCache } from '$lib/stores/media-cache.svelte';

	const id = $derived(page.params.id ?? '');

	let item = $state<CatalogItem | null>(null);
	let plan = $state<PlaybackPlan | null>(null);
	let progress = $state<Progress | null>(null);
	// The title's other files, listed in the sidebar so the viewer can
	// switch episodes without going back to the title page.
	let episodes = $state<CatalogItem[]>([]);
	let progressMap = $state<Map<string, Progress>>(new Map());
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);
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
		void load();
	});

	function stillUrl(episodeId: string): string {
		return api.thumbnail.url(episodeId, session.token, { width: 320 });
	}

	function progressLine(episode: CatalogItem): string {
		const p = progressMap.get(episode.id);
		if (!p) return 'Not watched';
		if (p.completed) return 'Watched';
		if (p.duration_sec > 0)
			return `Resume at ${Math.min(100, Math.round((p.position_sec / p.duration_sec) * 100))}%`;
		return 'Not watched';
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
	<div class="flex h-dvh flex-col lg:flex-row">
		<div class="relative min-h-0 min-w-0 flex-1">
			{#if plan.available}
				<!-- A new episode is a new session: keying on the item rebuilds
				     the player (video element, transcode session, clock) instead
				     of leaving the previous episode's state behind. -->
				{#key item.id}
					<Player {item} {plan} initialProgress={progress} />
				{/key}
			{:else}
				<div class="relative flex h-full items-center justify-center overflow-hidden bg-background px-6">
					<div class="lattice absolute inset-0 opacity-30"></div>
					<div class="relative max-w-lg text-center">
						<MonitorPlay class="mx-auto size-10 text-muted" />
						<h1 class="mt-5 text-xl font-semibold text-foreground">{item.title}</h1>
						<p class="mt-3 text-sm leading-relaxed text-muted">
							{plan.reason ?? 'No playback plan is available for this file in the browser.'}
						</p>
						<p class="mt-3 text-xs leading-relaxed text-muted/80">
							The server only sends bytes it can actually serve: the transcoder is unavailable, so
							there is no browser-playable version of this container. The desktop/CLI client plays
							it directly.
						</p>
						<div class="mt-7 flex flex-wrap items-center justify-center gap-2">
							<Button variant="secondary" onclick={() => void goto(detailHref)}>
								<ArrowLeft class="size-4" /> Back to details
							</Button>
							<Button variant="ghost" onclick={() => void goto('/library')}>Browse library</Button>
						</div>
					</div>
				</div>
			{/if}
		</div>

		{#if episodes.length > 1}
			<aside
				class="hidden w-80 shrink-0 flex-col border-l border-line/40 bg-surface/60 lg:flex"
				aria-label="Episodes"
			>
				<div class="border-b border-line/40 px-4 py-3">
					<p class="truncate text-sm font-semibold text-foreground">{item.title}</p>
					<p class="mt-0.5 text-xs text-muted">{episodes.length} episodes</p>
				</div>
				<ul class="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
					{#each episodes as episode (episode.id)}
						{@const current = episode.id === item.id}
						<li>
							<a
								href={`/player/${episode.id}`}
								aria-current={current ? 'true' : undefined}
								aria-label={`${episodeLabel(episode)}${current ? ', playing' : ''}`}
								class={[
									'flex items-center gap-3 p-2 transition-colors',
									current ? 'bg-foreground/5' : 'hover:bg-foreground/8'
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
								</span>
								<span class="min-w-0 flex-1">
									<span
										class={[
											'block font-mono text-[10px] uppercase tracking-[0.14em]',
											current ? 'text-accent' : 'text-muted'
										].join(' ')}
									>
										{episodeLabel(episode)}
									</span>
									<span class="mt-0.5 block truncate text-xs text-muted">{progressLine(episode)}</span>
								</span>
							</a>
						</li>
					{/each}
				</ul>
			</aside>
		{/if}
	</div>
{/if}
