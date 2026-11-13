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
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
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
	// What stands where the picture would be: a frame of the title itself.
	const poster = $derived(itemId ? api.thumbnail.url(itemId, session.token, { at: 10, width: 960 }) : '');
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
		<!-- A paused player: a frame of the title, and Play. The whole picture is the button. -->
		<button
			type="button"
			class="group relative min-h-0 w-full flex-1 overflow-hidden bg-black text-left max-md:portrait:aspect-video max-md:portrait:flex-none"
			aria-label="Play {title}"
			onclick={() => (playing = true)}
		>
			{#if poster}
				<img
					src={poster}
					alt=""
					draggable="false"
					class="absolute inset-0 size-full object-cover transition-transform duration-700 ease-out group-hover:scale-[1.03]"
				/>
			{/if}
			<span class="absolute inset-0 bg-gradient-to-t from-black/85 via-black/20 to-black/40" aria-hidden="true"></span>
			<span
				class="absolute left-1/2 top-1/2 grid size-16 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full bg-accent text-accent-fg shadow-[0_0_0_10px_rgb(255_255_255/0.08)] transition-transform duration-200 group-hover:scale-110 sm:size-20"
				aria-hidden="true"
			>
				<Play class="ml-1 size-7 sm:size-8" fill="currentColor" />
			</span>
			<span class="absolute inset-x-0 bottom-0 flex flex-col gap-1 px-4 pb-3 sm:px-6 sm:pb-5">
				<span class="font-mono text-[10px] uppercase tracking-[0.28em] text-accent">Live demo</span>
				<span class="max-w-xl text-xs leading-snug text-white/80 sm:text-sm">
					No files behind this demo, so {title} cannot play here. Press Play anyway.
				</span>
			</span>
		</button>
	{/if}
</div>
