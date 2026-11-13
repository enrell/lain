<script lang="ts">
	// The site's live demo has no media behind it. Where the player would be,
	// it offers a well-known video from its publisher's own channel, loaded
	// only when asked for (nothing is requested from another site before).
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import PanelRightClose from '@lucide/svelte/icons/panel-right-close';
	import PanelRightOpen from '@lucide/svelte/icons/panel-right-open';
	import Play from '@lucide/svelte/icons/play';
	// Import the hash-router adapter directly: vite.config.ts skips rewriting
	// `$app/navigation` for files inside `src/lib/demo/`.
	import { goto } from './navigation';
	import { t } from '$lib/i18n';

	interface EpisodesControl {
		readonly shown: boolean;
		readonly overlay: boolean;
		toggle: () => void;
	}

	let {
		itemId,
		title,
		episodes = null
	}: {
		itemId?: string;
		title: string;
		episodes?: EpisodesControl | null;
	} = $props();

	let playing = $state(false);
	const PRIMARY_VIDEO = 'dQw4w9WgXcQ';
	// YouTube rejects official music-label embeds like dQw4w9WgXcQ when Referer
	// uses a raw IP address (e.g. 127.0.0.1 local previews); fall back to an
	// open Blender Studio film embed so Play anyway still works on IP hosts.
	const IP_FALLBACK_VIDEO = 'aqz-KE-bpKQ';
	const isIpHost = (host: string): boolean => host.includes(':') || /^(\d{1,3}\.){3}\d{1,3}$/.test(host);
	const videoId = $derived(
		typeof window !== 'undefined' && isIpHost(window.location.hostname)
			? IP_FALLBACK_VIDEO
			: PRIMARY_VIDEO
	);
	const backHref = $derived(itemId ? `/item/${itemId}` : '/library');
	const episodesLabel = $derived(
		episodes ? (episodes.shown ? t('player.episodesHide') : t('player.episodesShow')) : ''
	);
</script>

<div class="flex h-full w-full flex-col bg-black max-md:portrait:h-auto">
	<header class="flex h-12 shrink-0 items-center justify-between gap-3 border-b border-white/5 px-3 sm:h-16 sm:gap-4 sm:px-6">
		<div class="flex min-w-0 items-center gap-2 sm:gap-4">
			<button
				type="button"
				class="inline-flex min-h-11 shrink-0 items-center gap-2 rounded-md px-2 text-sm text-foreground transition-colors hover:bg-surface-hover/60"
				onclick={() => void goto(backHref)}
			>
				<ArrowLeft class="size-5" aria-hidden="true" /> {t('player.back')}
			</button>
			<p class="min-w-0 truncate text-base font-medium text-foreground sm:text-lg">{title}</p>
		</div>
		{#if episodes && (!episodes.shown || episodes.overlay)}
			<span class="contents" data-episodes-toggle>
				<button
					type="button"
					class="inline-flex min-h-10 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-xs text-muted transition-colors hover:bg-surface-hover/60 hover:text-foreground"
					aria-label={episodesLabel}
					title={episodesLabel}
					onclick={episodes.toggle}
				>
					{#if episodes.shown}
						<PanelRightClose class="size-5" aria-hidden="true" />
					{:else}
						<PanelRightOpen class="size-5" aria-hidden="true" />
					{/if}
					<span class="hidden sm:inline">{t('player.list.label')}</span>
				</button>
			</span>
		{/if}
	</header>

	{#if playing}
		<div class="relative min-h-0 w-full flex-1 bg-black max-md:portrait:flex-none max-md:portrait:aspect-video">
			<iframe
				class="absolute inset-0 size-full"
				src="https://www.youtube-nocookie.com/embed/{videoId}?autoplay=1&playsinline=1&rel=0"
				title="A video the demo plays instead of {title}"
				allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
				allowfullscreen
				referrerpolicy="strict-origin-when-cross-origin"
			></iframe>
		</div>
	{:else}
		<div class="flex min-h-0 w-full flex-1 flex-col overflow-y-auto px-5 py-6 text-center max-md:portrait:flex-none sm:px-8 sm:py-10">
			<div class="mx-auto my-auto flex max-w-md flex-col items-center gap-3.5 sm:gap-5">
				<p class="font-mono text-[10px] uppercase tracking-[0.28em] text-accent">Live demo</p>
				<p class="text-base font-semibold leading-snug text-foreground sm:text-lg">
					This demo has no files behind it, so {title} cannot play here.
				</p>
				<p class="text-xs leading-relaxed text-muted sm:text-sm">
					On your own server this is the player: direct play, subtitles, seek previews. Here, Play brings something else.
				</p>
				<button
					type="button"
					class="mt-1 inline-flex h-11 shrink-0 items-center gap-2 rounded-lg bg-accent px-5 text-sm font-semibold text-accent-fg transition-colors hover:bg-accent-hover sm:h-12 sm:px-6"
					onclick={() => (playing = true)}
				>
					<Play class="size-4" fill="currentColor" /> Play anyway
				</button>
			</div>
		</div>
	{/if}
</div>
