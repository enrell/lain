<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { DropdownMenu, Slider } from 'bits-ui';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Gauge from '@lucide/svelte/icons/gauge';
	import Maximize from '@lucide/svelte/icons/maximize';
	import Minimize from '@lucide/svelte/icons/minimize';
	import Pause from '@lucide/svelte/icons/pause';
	import Play from '@lucide/svelte/icons/play';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import SkipBack from '@lucide/svelte/icons/skip-back';
	import SkipForward from '@lucide/svelte/icons/skip-forward';
	import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
	import Volume1 from '@lucide/svelte/icons/volume-1';
	import Volume2 from '@lucide/svelte/icons/volume-2';
	import VolumeX from '@lucide/svelte/icons/volume-x';
	import type { CatalogItem, PlaybackPlan, Progress } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import IconButton from '$lib/components/primitives/IconButton.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { prefs } from '$lib/auth/storage';
	import { isCompleted } from '$lib/utilities/progress';
	import { ProgressReporter, type ProgressSnapshot } from '$lib/utilities/progress-reporter';
	import { formatTime } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';

	let {
		item,
		plan,
		initialProgress
	}: {
		item: CatalogItem;
		plan: PlaybackPlan;
		initialProgress: Progress | null;
	} = $props();

	const RATES = [0.5, 0.75, 1, 1.25, 1.5, 2];
	const HIDE_DELAY_MS = 2600;

	let container = $state<HTMLDivElement | null>(null);
	let video = $state<HTMLVideoElement | null>(null);

	let playing = $state(false);
	let waiting = $state(false);
	let ended = $state(false);
	let currentTime = $state(0);
	let duration = $state(0);
	let buffered = $state(0);
	let volume = $state(prefs.getNumber('player.volume', 1));
	let muted = $state(prefs.getBool('player.muted', false));
	let rate = $state(prefs.getNumber('player.rate', 1));
	let fullscreen = $state(false);
	let controlsVisible = $state(true);
	let scrubbing = $state(false);
	let scrubValue = $state(0);
	let error = $state<string | null>(null);
	let resumedFrom = $state(0);
	let showResume = $state(false);

	let hideTimer: ReturnType<typeof setTimeout> | null = null;
	let resumeTimer: ReturnType<typeof setTimeout> | null = null;
	let resumeApplied = false;

	const streamSrc = $derived(api.playback.streamUrl(item.id, session.token));
	const title = $derived(item.title);

	function snapshot(): ProgressSnapshot {
		const el = video;
		const pos = el?.currentTime ?? 0;
		const dur = el?.duration && Number.isFinite(el.duration) ? el.duration : 0;
		return { position_sec: pos, duration_sec: dur, completed: isCompleted(pos, dur) };
	}

	/*
	 * Bounded writes: timeupdate only proposes a snapshot; the reporter
	 * decides. Lifecycle transitions force an immediate write, and a
	 * failed write is kept for the next attempt instead of surfacing.
	 */
	const reporter = new ProgressReporter(
		(snap) => api.progress.put(item.id, snap).then(() => undefined),
		{
			onError: (err) => {
				if (import.meta.env.DEV) console.warn('[lain-player] progress write failed', err);
			}
		}
	);

	function persistNow(): void {
		const snap = snapshot();
		if (snap.duration_sec <= 0) return;
		void api.progress.put(item.id, snap, { keepalive: true }).catch(() => {
			// The page is going away; there is no UI left to inform.
		});
	}

	function onPageHide(): void {
		if (document.visibilityState === 'hidden') persistNow();
	}

	onMount(() => {
		window.addEventListener('pagehide', onPageHide);
		document.addEventListener('visibilitychange', onPageHide);
		return () => {
			window.removeEventListener('pagehide', onPageHide);
			document.removeEventListener('visibilitychange', onPageHide);
		};
	});

	onDestroy(() => {
		persistNow();
		if (hideTimer) clearTimeout(hideTimer);
		if (resumeTimer) clearTimeout(resumeTimer);
	});

	/* ---------------------------------------------------------------- */
	/* media element events                                              */
	/* ---------------------------------------------------------------- */

	function onLoadedMetadata(): void {
		const el = video;
		if (!el) return;
		duration = Number.isFinite(el.duration) ? el.duration : 0;
		el.volume = volume;
		el.muted = muted;
		el.playbackRate = rate;
		if (!resumeApplied) {
			resumeApplied = true;
			const resume =
				initialProgress && !initialProgress.completed && initialProgress.position_sec >= 5
					? initialProgress.position_sec
					: 0;
			if (resume > 0 && duration > 0 && resume < duration - 5) {
				el.currentTime = resume;
				resumedFrom = resume;
				showResume = true;
				resumeTimer = setTimeout(() => (showResume = false), 7000);
			}
		}
	}

	function onDurationChange(): void {
		const el = video;
		if (el && Number.isFinite(el.duration)) duration = el.duration;
	}

	function onTimeUpdate(): void {
		const el = video;
		if (!el || scrubbing) return;
		currentTime = el.currentTime;
		if (el.buffered.length > 0 && el.duration > 0) {
			buffered = el.buffered.end(el.buffered.length - 1) / el.duration;
		}
		void reporter.update(snapshot());
	}

	function onPause(): void {
		playing = false;
		waiting = false;
		controlsVisible = true;
		void reporter.update(snapshot(), { force: true });
	}

	function onPlay(): void {
		playing = true;
		ended = false;
		error = null;
		scheduleHide();
	}

	function onPlaying(): void {
		waiting = false;
	}

	function onWaiting(): void {
		if (playing) waiting = true;
	}

	function onSeeked(): void {
		if (video) currentTime = video.currentTime;
		void reporter.update(snapshot(), { force: true });
	}

	function onEnded(): void {
		playing = false;
		ended = true;
		controlsVisible = true;
		void reporter.update({ ...snapshot(), completed: true }, { force: true });
	}

	function onMediaError(): void {
		playing = false;
		waiting = false;
		const code = video?.error?.code;
		switch (code) {
			case MediaError.MEDIA_ERR_NETWORK:
				error = 'The stream connection dropped.';
				break;
			case MediaError.MEDIA_ERR_DECODE:
				error = 'This file could not be decoded by the browser.';
				break;
			case MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED:
				error = 'This media format is not supported in the browser.';
				break;
			default:
				error = 'Playback failed.';
		}
	}

	function retryPlayback(): void {
		error = null;
		video?.load();
		void video?.play().catch(() => undefined);
	}

	/* ---------------------------------------------------------------- */
	/* controls                                                          */
	/* ---------------------------------------------------------------- */

	function scheduleHide(): void {
		if (hideTimer) clearTimeout(hideTimer);
		if (!playing || scrubbing) {
			controlsVisible = true;
			return;
		}
		hideTimer = setTimeout(() => {
			hideTimer = null;
			if (playing && !scrubbing) controlsVisible = false;
		}, HIDE_DELAY_MS);
	}

	function revealControls(): void {
		controlsVisible = true;
		scheduleHide();
	}

	function togglePlay(): void {
		const el = video;
		if (!el) return;
		if (el.paused || el.ended) {
			ended = false;
			void el.play().catch(() => {
				error = 'The browser blocked playback. Click play to start.';
			});
		} else {
			el.pause();
		}
	}

	function seekBy(seconds: number): void {
		const el = video;
		if (!el || !Number.isFinite(el.duration)) return;
		el.currentTime = Math.max(0, Math.min(el.duration, el.currentTime + seconds));
		currentTime = el.currentTime;
		revealControls();
	}

	function seekTo(seconds: number): void {
		const el = video;
		if (!el || !Number.isFinite(el.duration)) return;
		el.currentTime = Math.max(0, Math.min(el.duration, seconds));
		currentTime = el.currentTime;
	}

	function setVolume(next: number): void {
		volume = Math.max(0, Math.min(1, next));
		if (video) {
			video.volume = volume;
			video.muted = false;
		}
		muted = false;
		prefs.set('player.volume', String(volume));
		prefs.set('player.muted', 'false');
	}

	function toggleMute(): void {
		muted = !muted;
		if (video) video.muted = muted;
		prefs.set('player.muted', String(muted));
	}

	function setRate(next: number): void {
		rate = next;
		if (video) video.playbackRate = next;
		prefs.set('player.rate', String(next));
	}

	async function toggleFullscreen(): Promise<void> {
		const el = container;
		if (!el) return;
		try {
			if (document.fullscreenElement) {
				await document.exitFullscreen();
			} else {
				await el.requestFullscreen();
			}
		} catch {
			// Fullscreen can be denied (permissions, iOS): the video
			// element's native fullscreen remains available.
		}
	}

	function onFullscreenChange(): void {
		fullscreen = document.fullscreenElement !== null;
	}

	onMount(() => {
		document.addEventListener('fullscreenchange', onFullscreenChange);
		return () => document.removeEventListener('fullscreenchange', onFullscreenChange);
	});

	function skipResumeToStart(): void {
		if (video) video.currentTime = 0;
		showResume = false;
		if (resumeTimer) clearTimeout(resumeTimer);
		resumedFrom = 0;
	}

	/* ---------------------------------------------------------------- */
	/* keyboard                                                          */
	/* ---------------------------------------------------------------- */

	function shortcutsAllowed(event: KeyboardEvent): boolean {
		if (event.metaKey || event.ctrlKey || event.altKey) return false;
		const target = event.target;
		if (isTypingTarget(target)) return false;
		if (target instanceof HTMLElement) {
			const tag = target.tagName;
			if (tag === 'BUTTON' || tag === 'A' || tag === 'SELECT') return false;
			if (target.getAttribute('role') === 'slider') return false;
		}
		return true;
	}

	function onKeydown(event: KeyboardEvent): void {
		if (!shortcutsAllowed(event)) return;
		switch (event.key) {
			case ' ':
			case 'k':
			case 'K':
				event.preventDefault();
				togglePlay();
				revealControls();
				break;
			case 'ArrowLeft':
				event.preventDefault();
				seekBy(-5);
				break;
			case 'ArrowRight':
				event.preventDefault();
				seekBy(5);
				break;
			case 'j':
			case 'J':
				seekBy(-10);
				break;
			case 'l':
			case 'L':
				seekBy(10);
				break;
			case 'ArrowUp':
				event.preventDefault();
				setVolume(volume + 0.05);
				revealControls();
				break;
			case 'ArrowDown':
				event.preventDefault();
				setVolume(volume - 0.05);
				revealControls();
				break;
			case 'm':
			case 'M':
				toggleMute();
				revealControls();
				break;
			case 'f':
			case 'F':
				void toggleFullscreen();
				break;
			case 'Escape':
				if (!document.fullscreenElement) void goto(`/item/${item.id}`);
				break;
		}
	}

	const VolumeIcon = $derived(muted || volume === 0 ? VolumeX : volume < 0.5 ? Volume1 : Volume2);
</script>

<svelte:window onkeydown={onKeydown} />

<div
	bind:this={container}
	class="relative h-dvh w-full overflow-hidden bg-black"
	role="region"
	aria-label={`Player — ${title}`}
	onpointermove={revealControls}
	onpointerdown={revealControls}
>
	<video
		bind:this={video}
		src={streamSrc}
		class="size-full object-contain"
		preload="metadata"
		playsinline
		onclick={togglePlay}
		onloadedmetadata={onLoadedMetadata}
		ondurationchange={onDurationChange}
		ontimeupdate={onTimeUpdate}
		onpause={onPause}
		onplay={onPlay}
		onplaying={onPlaying}
		onwaiting={onWaiting}
		onseeked={onSeeked}
		onended={onEnded}
		onerror={onMediaError}
	>
		<!-- No captions are transcribed yet; the element keeps native
		     accessibility semantics without inventing fake tracks. -->
		<track kind="captions" />
	</video>

	<!-- Buffering -->
	{#if waiting && !error}
		<div class="pointer-events-none absolute inset-0 flex items-center justify-center">
			<Spinner class="size-10 text-white/80" label="Buffering" />
		</div>
	{/if}

	<!-- Error -->
	{#if error}
		<div class="absolute inset-0 flex items-center justify-center bg-black/80 px-6">
			<div class="max-w-md text-center">
				<TriangleAlert class="mx-auto size-8 text-warning" />
				<p class="mt-3 text-sm text-foreground">{error}</p>
				<div class="mt-5 flex justify-center gap-2">
					<button
						class="rounded-md bg-accent px-4 py-2 text-sm font-medium text-accent-fg hover:bg-accent-hover"
						onclick={retryPlayback}
					>
						<RotateCcw class="mr-1.5 inline size-3.5" /> Retry
					</button>
					<a
						href={`/item/${item.id}`}
						class="rounded-md border border-line bg-surface px-4 py-2 text-sm text-foreground hover:bg-surface-hover"
					>
						Back to details
					</a>
				</div>
			</div>
		</div>
	{/if}

	<!-- Paused start / ended: a large, unmissable play affordance. -->
	{#if (ended || (!playing && !error && !waiting && currentTime === 0)) && controlsVisible}
		<button
			class="absolute inset-0 flex items-center justify-center"
			onclick={togglePlay}
			aria-label={ended ? 'Play again' : 'Play'}
		>
			<span
				class="flex size-20 items-center justify-center rounded-full bg-black/55 text-foreground backdrop-blur transition-transform duration-150 hover:scale-105"
			>
				<Play class="size-9 translate-x-0.5" />
			</span>
		</button>
	{/if}

	<!-- Resume banner -->
	{#if showResume && resumedFrom > 0}
		<div
			class="absolute left-1/2 top-16 -translate-x-1/2 rounded-card border border-line bg-black/75 px-4 py-3 text-sm text-foreground backdrop-blur"
		>
			Resuming from {formatTime(resumedFrom)}
			<button class="ml-3 text-xs font-medium text-accent hover:text-accent-hover" onclick={skipResumeToStart}>
				Start over
			</button>
		</div>
	{/if}

	<!-- Chrome -->
	<div
		class={[
			'absolute inset-x-0 top-0 flex items-center gap-3 bg-gradient-to-b from-black/70 to-transparent p-4 transition-opacity duration-200',
			controlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0'
		].join(' ')}
	>
		<IconButton label="Back to details" class="text-white/80 hover:text-white" onclick={() => void goto(`/item/${item.id}`)}>
			<ArrowLeft class="size-5" />
		</IconButton>
		<p class="min-w-0 truncate text-sm font-medium text-white/90">{title}</p>
	</div>

	<div
		class={[
			'absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/85 via-black/50 to-transparent px-4 pb-4 pt-16 transition-opacity duration-200',
			controlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0'
		].join(' ')}
	>
		<!-- Seek -->
		<div class="group/seek flex items-center gap-3">
			<Slider.Root
				type="single"
				value={scrubbing ? scrubValue : currentTime}
				max={duration || 0}
				step={0.1}
				onValueChange={(v) => {
					scrubbing = true;
					scrubValue = v;
				}}
				onValueCommit={(v) => {
					seekTo(v);
					scrubbing = false;
				}}
				class="relative flex h-6 w-full touch-none select-none items-center"
			>
				<span class="relative h-1.5 w-full grow overflow-hidden rounded-full bg-white/20">
					<span class="absolute inset-y-0 left-0 rounded-full bg-white/25" style={`width:${Math.min(100, buffered * 100)}%`}></span>
					<Slider.Range class="absolute h-full rounded-full bg-accent" />
				</span>
				<Slider.Thumb
					index={0}
					aria-label="Seek"
					class="block size-3.5 rounded-full bg-accent shadow transition-transform duration-150 hover:scale-125 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
				/>
			</Slider.Root>
			<span class="shrink-0 font-mono text-xs tabular-nums text-white/80">
				{formatTime(scrubbing ? scrubValue : currentTime)} / {formatTime(duration)}
			</span>
		</div>

		<!-- Transport -->
		<div class="mt-1 flex items-center gap-1.5">
			<IconButton label="Back 10 seconds" class="text-white/85 hover:text-white" onclick={() => seekBy(-10)}>
				<SkipBack class="size-5" />
			</IconButton>
			<IconButton label={playing ? 'Pause' : 'Play'} class="text-white hover:text-white" onclick={togglePlay}>
				{#if playing}<Pause class="size-6" />{:else}<Play class="size-6" />{/if}
			</IconButton>
			<IconButton label="Forward 10 seconds" class="text-white/85 hover:text-white" onclick={() => seekBy(10)}>
				<SkipForward class="size-5" />
			</IconButton>

			<div class="ml-2 hidden items-center gap-1.5 md:flex">
				<IconButton label={muted ? 'Unmute' : 'Mute'} class="text-white/85 hover:text-white" onclick={toggleMute}>
					<VolumeIcon class="size-5" />
				</IconButton>
				<div class="w-24">
					<Slider.Root
						type="single"
						value={muted ? 0 : volume}
						max={1}
						step={0.02}
						onValueChange={setVolume}
						class="relative flex h-6 touch-none select-none items-center"
					>
						<span class="relative h-1 w-full grow overflow-hidden rounded-full bg-white/20">
							<Slider.Range class="absolute h-full rounded-full bg-white" />
						</span>
						<Slider.Thumb
							index={0}
							aria-label="Volume"
							class="block size-3 rounded-full bg-white focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
						/>
					</Slider.Root>
				</div>
			</div>

			<div class="ml-auto flex items-center gap-1.5">
				<DropdownMenu.Root>
					<DropdownMenu.Trigger
						class="inline-flex h-9 items-center gap-1.5 rounded-md px-2 text-xs font-medium text-white/85 hover:bg-white/10 hover:text-white"
						aria-label={`Playback speed ${rate}x`}
					>
						<Gauge class="size-4" /> {rate}x
					</DropdownMenu.Trigger>
					<DropdownMenu.Portal>
						<DropdownMenu.Content
							side="top"
							align="end"
							sideOffset={8}
							class="z-50 min-w-28 rounded-md border border-line bg-surface p-1 shadow-xl"
						>
							{#each RATES as option (option)}
								<DropdownMenu.Item
									class="flex cursor-default items-center justify-between rounded-sm px-2.5 py-1.5 text-sm text-foreground outline-none data-[highlighted]:bg-surface-hover"
									onSelect={() => setRate(option)}
								>
									{option}x {#if option === rate}<span class="text-accent">•</span>{/if}
								</DropdownMenu.Item>
							{/each}
						</DropdownMenu.Content>
					</DropdownMenu.Portal>
				</DropdownMenu.Root>
				<IconButton
					label={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
					class="text-white/85 hover:text-white"
					onclick={() => void toggleFullscreen()}
				>
					{#if fullscreen}<Minimize class="size-5" />{:else}<Maximize class="size-5" />{/if}
				</IconButton>
			</div>
		</div>
	</div>
</div>
