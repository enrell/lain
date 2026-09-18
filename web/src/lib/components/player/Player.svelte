<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { DropdownMenu, Slider } from 'bits-ui';
	import Hls from 'hls.js';
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
	import type {
		CatalogItem,
		PlaybackOptions,
		PlaybackPlan,
		Progress,
		TranscodeDelivery,
		TranscodeStatus
	} from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import IconButton from '$lib/components/primitives/IconButton.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { prefs } from '$lib/auth/storage';
	import { isCompleted } from '$lib/utilities/progress';
	import { ProgressReporter, type ProgressSnapshot } from '$lib/utilities/progress-reporter';
	import { elapsedSince, formatTime } from '$lib/utilities/format';
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
	// True only between pointerdown and pointerup on the seek row. Used to
	// tell a real drag from a stray `onValueChange` echo (see below).
	let seekEngaged = $state(false);
	let error = $state<string | null>(null);
	let resumedFrom = $state(0);
	let showResume = $state(false);
	let transcodeReady = $state(false);
	let preparingTranscode = $state(false);
	let transcodeSession = $state('');
	let transcodeError = $state<string | null>(null);
	let prepareProgress = $state(0);
	let prepareStartedAt = $state(0);
	let nowMs = $state(Date.now());
	let hasSubtitle = $state(false);
	let selectedAudio = $state('');
	let selectedSubtitle = $state('');
	// v3 session state (D-042): delivery actually used, the quality the
	// viewer picked, why the server transcodes, and the hls.js instance.
	let playbackOptions = $state<PlaybackOptions | null>(null);
	let selectedQuality = $state('');
	let delivery = $state<TranscodeDelivery | ''>('');
	let transcodeReasons = $state<string[]>([]);
	// baseOffset is the source position the current session starts at (a
	// resume/seek). The session's own timeline is zero-based, so the
	// displayed clock and the progress report add it back.
	let baseOffset = $state(0);
	let hls: Hls | null = null;
	// Set on unmount so a late status/attach continuation cannot create an
	// Hls instance after destroyHLS() has already run.
	let destroyed = false;

	const audioTracks = $derived((plan.streams ?? []).filter((s) => s.type === 'audio'));
	const subtitleTracks = $derived(
		(plan.streams ?? []).filter((s) => s.type === 'subtitle' && s.convertible)
	);
	// Transcode sessions serve the sidecar the session prepared; direct
	// play asks the server to extract the selected track on the fly.
	const subtitleSrc = $derived(
		plan.mode === 'transcode' && transcodeReady && hasSubtitle && transcodeSession
			? api.playback.subtitleUrl(item.id, session.token, transcodeSession)
			: plan.mode !== 'transcode' && selectedSubtitle !== ''
				? api.playback.streamSubtitleUrl(item.id, session.token, Number(selectedSubtitle))
				: undefined
	);

	let hideTimer: ReturnType<typeof setTimeout> | null = null;
	let resumeTimer: ReturnType<typeof setTimeout> | null = null;
	let resumeApplied = false;
	let transcodeAbort: AbortController | null = null;

	const streamSrc = $derived(
		plan.mode === 'transcode' && delivery === 'hls'
			? undefined
			: plan.mode === 'transcode' && !transcodeReady
				? undefined
				: plan.mode === 'transcode'
					? api.playback.transcodeUrl(item.id, session.token, transcodeSession)
					: api.playback.streamUrl(item.id, session.token)
	);
	const title = $derived(item.title);

	function resumePosition(): number {
		if (!initialProgress || initialProgress.completed) return 0;
		return initialProgress.position_sec >= 5 ? initialProgress.position_sec : 0;
	}

	function snapshot(): ProgressSnapshot {
		const el = video;
		const pos = (el?.currentTime ?? 0) + baseOffset;
		const dur = el?.duration && Number.isFinite(el.duration) ? el.duration + baseOffset : 0;
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
		void loadPlaybackOptions();
		if (plan.mode === 'transcode') {
			transcodeSession = plan.session ?? '';
			transcodeReasons = plan.reasons ?? [];
			if (plan.state === 'ready') {
				// A cached session was produced from the start of the source.
				void adoptReadySession();
			} else {
				// Start a fresh session at the saved position so a resume does
				// not re-encode from the beginning.
				baseOffset = resumePosition();
				void prepareTranscode();
			}
		} else {
			transcodeReady = true;
		}
		return () => {
			window.removeEventListener('pagehide', onPageHide);
			document.removeEventListener('visibilitychange', onPageHide);
		};
	});

	onDestroy(() => {
		destroyed = true;
		persistNow();
		if (hideTimer) clearTimeout(hideTimer);
		if (resumeTimer) clearTimeout(resumeTimer);
		transcodeAbort?.abort();
		destroyHLS();
	});

	async function loadPlaybackOptions(): Promise<void> {
		try {
			playbackOptions = await api.playback.options();
		} catch {
			// The player still works without the ladder: Auto only.
		}
	}

	// HLS is attached through hls.js (MSE) or the browser's native HLS
	// (Safari); the server-owned playlist is the only URL involved.
	function attachHLS(): void {
		const el = video;
		if (!el || !transcodeSession) return;
		const url = api.playback.hlsUrl(item.id, session.token, transcodeSession, 'index.m3u8');
		destroyHLS();
		if (Hls.isSupported()) {
			hls = new Hls({ enableWorker: true });
			let mediaRecoveries = 0;
			hls.on(Hls.Events.ERROR, (_event, data) => {
				if (!data.fatal) return;
				// One media recovery is the documented remedy; a second fatal
				// media error means the stream is broken, so report it instead
				// of looping on recoverMediaError forever.
				if (data.type === Hls.ErrorTypes.MEDIA_ERROR && mediaRecoveries < 1) {
					mediaRecoveries++;
					hls?.recoverMediaError();
					return;
				}
				error = 'HLS playback failed.';
			});
			hls.loadSource(url);
			hls.attachMedia(el);
		} else if (el.canPlayType('application/vnd.apple.mpegurl')) {
			el.src = url;
		} else {
			error = 'This browser cannot play HLS streams.';
		}
	}

	function destroyHLS(): void {
		hls?.destroy();
		hls = null;
	}

	function delay(ms: number, signal: AbortSignal): Promise<void> {
		return new Promise((resolve, reject) => {
			const timer = setTimeout(resolve, ms);
			signal.addEventListener(
				'abort',
				() => {
					clearTimeout(timer);
					reject(new DOMException('Aborted', 'AbortError'));
				},
				{ once: true }
			);
		});
	}

	// One status reading feeds both the loop and the overlay: the server
	// owns started_at, so a reload never restarts the elapsed story.
	function applyPrepareStatus(status: TranscodeStatus): void {
		prepareProgress = status.progress ?? 0;
		prepareStartedAt = status.started_at || status.queued_at || prepareStartedAt;
		nowMs = Date.now();
	}

	// A cached session is already prepared; learn its delivery and sidecar
	// from the status instead of restarting preparation. A cached HLS
	// session must be attached through hls.js, not fetched as a
	// progressive MP4 (the gateway rejects that with 409).
	async function adoptReadySession(): Promise<void> {
		if (!transcodeSession) {
			await prepareTranscode();
			return;
		}
		try {
			const status = await api.playback.transcodeStatus(item.id, transcodeSession);
			if (destroyed) return;
			delivery = status.delivery ?? 'progressive';
			hasSubtitle = status.has_subtitle ?? false;
			transcodeReasons = status.reasons ?? transcodeReasons;
			if (delivery === 'hls') attachHLS();
			transcodeReady = true;
		} catch {
			// The cached session is gone: fall back to preparation.
			await prepareTranscode();
		}
	}

	async function prepareTranscode(selection?: {
		audio_stream?: number;
		subtitle_stream?: number;
		quality?: string;
	}): Promise<void> {
		transcodeAbort?.abort();
		const controller = new AbortController();
		transcodeAbort = controller;
		preparingTranscode = true;
		transcodeError = null;
		prepareProgress = 0;
		prepareStartedAt = Date.now() / 1000;
		nowMs = Date.now();
		try {
			let status = await api.playback.startTranscode(item.id, {
				quality: (selection?.quality ?? selectedQuality) || undefined,
				audio_stream: selection?.audio_stream,
				subtitle_stream: selection?.subtitle_stream,
				start_sec: baseOffset || undefined
			});
			transcodeSession = status.session;
			delivery = status.delivery ?? 'progressive';
			transcodeReasons = status.reasons ?? transcodeReasons;
			hasSubtitle = status.has_subtitle ?? selection?.subtitle_stream !== undefined;
			applyPrepareStatus(status);
			let waitMs = 750;
			// HLS starts playing as soon as the playlist is playable; the
			// progressive path still waits for the complete MP4.
			const pending = () => status.state === 'queued' || status.state === 'running' || status.state === 'idle';
			const wantsMore = () => (delivery === 'hls' ? pending() && !status.playable : pending());
			while (wantsMore()) {
				await delay(waitMs, controller.signal);
				status = await api.playback.transcodeStatus(item.id, transcodeSession, controller.signal);
				applyPrepareStatus(status);
				waitMs = Math.min(3000, Math.round(waitMs * 1.4));
			}
			if (destroyed) return;
			if (status.state === 'failed') {
				throw new Error(status.error || 'The server could not prepare this video.');
			}
			if (delivery === 'hls') {
				if (!status.playable && status.state !== 'ready') {
					throw new Error(status.error || 'The server could not prepare this video.');
				}
				attachHLS();
				transcodeReady = true;
			} else {
				if (status.state !== 'ready') {
					throw new Error(status.error || 'The server could not prepare this video.');
				}
				hasSubtitle = status.has_subtitle ?? hasSubtitle;
				transcodeReady = true;
			}
		} catch (err) {
			if (err instanceof DOMException && err.name === 'AbortError') return;
			transcodeError = err instanceof Error ? err.message : 'The server could not prepare this video.';
		} finally {
			if (transcodeAbort === controller) {
				preparingTranscode = false;
				transcodeAbort = null;
			}
		}
	}

	// A direct-play subtitle choice only swaps the sidecar; a transcode
	// choice rebuilds the session, because the track is baked into it.
	function onSubtitleChoice(): void {
		if (plan.mode === 'transcode') void changeTracks();
	}

	async function changeTracks(): Promise<void> {
		if (plan.mode !== 'transcode') return;
		// Rebuild the session from the current display position so the swap
		// keeps the viewer's place (the new session's timeline restarts at 0).
		const position = currentTime;
		const oldSession = transcodeSession;
		destroyHLS();
		transcodeReady = false;
		resumeApplied = true;
		baseOffset = position;
		if (oldSession) void api.playback.cancelTranscode(item.id, oldSession).catch(() => undefined);
		await prepareTranscode({
			audio_stream: selectedAudio === '' ? undefined : Number(selectedAudio),
			subtitle_stream: selectedSubtitle === '' ? undefined : Number(selectedSubtitle),
			quality: selectedQuality || undefined
		});
		if (video && transcodeReady) {
			video.currentTime = 0;
			void video.play().catch(() => undefined);
		}
	}

	/* ---------------------------------------------------------------- */
	/* media element events                                              */
	/* ---------------------------------------------------------------- */

	function onLoadedMetadata(): void {
		const el = video;
		if (!el || !transcodeReady) return;
		duration = Number.isFinite(el.duration) ? el.duration + baseOffset : 0;
		el.volume = volume;
		el.muted = muted;
		el.playbackRate = rate;
		if (!resumeApplied) {
			resumeApplied = true;
			if (baseOffset > 0) {
				// The session already starts at the resume point.
				resumedFrom = baseOffset;
				showResume = true;
				resumeTimer = setTimeout(() => (showResume = false), 7000);
				return;
			}
			const resume = resumePosition();
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
		if (el && Number.isFinite(el.duration)) duration = el.duration + baseOffset;
	}

	function onTimeUpdate(): void {
		const el = video;
		if (!el) return;
		/*
		 * `scrubbing` may only survive while a pointer is actually down on
		 * the seek row. Slider echoes the controlled value prop through
		 * `onValueChange` when it re-snaps it to the step grid, and an echo
		 * that latched `scrubbing` used to freeze the readout for the whole
		 * session; a stale latch heals here instead.
		 */
		if (scrubbing && !seekEngaged) scrubbing = false;
		if (scrubbing) return;
		currentTime = el.currentTime + baseOffset;
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
		if (video) currentTime = video.currentTime + baseOffset;
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
		// An HLS session lives in an hls.js/MSE instance, not the element's
		// src: reloading the element would detach it and never re-attach.
		if (plan.mode === 'transcode' && delivery === 'hls') {
			attachHLS();
			transcodeReady = true;
			return;
		}
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
		currentTime = el.currentTime + baseOffset;
		revealControls();
	}

	function seekTo(seconds: number): void {
		const el = video;
		if (!el || !Number.isFinite(el.duration)) return;
		el.currentTime = Math.max(0, Math.min(el.duration, seconds - baseOffset));
		currentTime = el.currentTime + baseOffset;
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

	// A live elapsed clock, but only while the preparation overlay is up.
	$effect(() => {
		if (!preparingTranscode) return;
		nowMs = Date.now();
		const timer = setInterval(() => (nowMs = Date.now()), 1000);
		return () => clearInterval(timer);
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
	// A zero max collapses the slider's step grid to a single step, so every
	// playback position would be snapped away and echoed back as a change.
	const seekMax = $derived(duration > 0 ? duration : 1);
	// Preparation is the one wait with no media element reporting for it, so
	// the overlay counts from the server's own start time and shows the
	// fraction when the server has one (D-038/D-039).
	const prepareElapsed = $derived(elapsedSince(prepareStartedAt, nowMs));
	const preparePercent = $derived(Math.round(prepareProgress * 100));
</script>

<svelte:window
	onkeydown={onKeydown}
	onpointerup={() => (seekEngaged = false)}
	onpointercancel={() => (seekEngaged = false)}
/>

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
		     accessibility semantics without inventing fake tracks. The key
		     block rebuilds the element when the sidecar changes, which is
		     what makes switching tracks during direct play reliable. -->
		{#if subtitleSrc}
			{#key subtitleSrc}
				<track kind="subtitles" srclang="en" label="Subtitles" src={subtitleSrc} default />
			{/key}
		{/if}
		<track kind="captions" />
	</video>

	{#if transcodeReady && (audioTracks.length > 1 || subtitleTracks.length > 0 || (playbackOptions?.qualities.length ?? 0) > 0 || plan.mode === 'transcode')}
		<div class="absolute left-4 top-16 z-10 flex flex-wrap gap-2">
			{#if plan.mode === 'transcode' && (playbackOptions?.qualities.length ?? 0) > 0}
				<label class="flex items-center gap-2 rounded-md bg-black/70 px-2 py-1 text-xs text-white/85">
					Quality
					<select
						class="bg-transparent text-xs text-white"
						bind:value={selectedQuality}
						onchange={() => void changeTracks()}
						aria-label="Transcode quality"
					>
						<option value="">Auto</option>
						{#each playbackOptions?.qualities ?? [] as quality (quality.name)}
							<option value={quality.name}>
								{quality.name}{quality.bitrate_kbps ? ` · ${Math.round(quality.bitrate_kbps / 1000)} Mbps` : ''}
							</option>
						{/each}
					</select>
				</label>
			{/if}
			{#if plan.mode === 'transcode' && audioTracks.length > 1}
				<label class="flex items-center gap-2 rounded-md bg-black/70 px-2 py-1 text-xs text-white/85">
					Audio
					<select
						class="bg-transparent text-xs text-white"
						bind:value={selectedAudio}
						onchange={() => void changeTracks()}
						aria-label="Audio track"
					>
						<option value="">Default</option>
						{#each audioTracks as track (track.index)}
							<option value={String(track.index)}>
								{track.language || track.title || `Track ${track.index}`} · {track.codec}
							</option>
						{/each}
					</select>
				</label>
			{/if}
			{#if subtitleTracks.length > 0}
				<label class="flex items-center gap-2 rounded-md bg-black/70 px-2 py-1 text-xs text-white/85">
					CC
					<select
						class="bg-transparent text-xs text-white"
						bind:value={selectedSubtitle}
						onchange={onSubtitleChoice}
						aria-label="Subtitle track"
					>
						<option value="">Off</option>
						{#each subtitleTracks as track (track.index)}
							<option value={String(track.index)}>
								{track.language || track.title || `Track ${track.index}`}
							</option>
						{/each}
					</select>
				</label>
			{/if}
		</div>
	{/if}

	{#if preparingTranscode}
		<div class="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-black/80 px-6 text-center">
			<Spinner class="size-10 text-white/80" label="Preparing playback" />
			<div class="max-w-md">
				<p class="text-sm text-white/85">Preparing a browser-compatible version…</p>
				<p class="mt-2 font-mono text-xs tabular-nums text-white/60">
					{formatTime(prepareElapsed)} elapsed
				</p>
				{#if prepareProgress > 0}
					<div
						class="mx-auto mt-4 h-1 w-56 overflow-hidden rounded-full bg-white/20"
						role="progressbar"
						aria-label="Preparation progress"
						aria-valuemin="0"
						aria-valuemax="100"
						aria-valuenow={preparePercent}
					>
						<div class="h-full rounded-full bg-accent" style={`width:${preparePercent}%`}></div>
					</div>
					<p class="mt-1.5 text-xs text-white/60">{preparePercent}% prepared</p>
				{/if}
				<p class="mt-4 text-xs leading-relaxed text-white/60">
					The first play prepares a browser-compatible copy of this file. Sources like AV1 or
					HEVC need a full re-encode and can take several minutes.
				</p>
				<p class="mt-2 text-xs leading-relaxed text-white/60">
					You can leave this page — preparation continues on the server.
				</p>
				<button
					class="mt-5 rounded-md border border-line bg-surface px-4 py-2 text-sm text-foreground hover:bg-surface-hover"
					onclick={() => void goto(`/item/${item.id}`)}
				>
					<ArrowLeft class="mr-1.5 inline size-3.5" /> Back to details
				</button>
			</div>
		</div>
	{:else if transcodeError}
		<div class="absolute inset-0 flex items-center justify-center bg-black/80 px-6" role="alert">
			<div class="max-w-md text-center">
				<TriangleAlert class="mx-auto size-8 text-warning" />
				<p class="mt-3 text-sm text-foreground">{transcodeError}</p>
				<button
					class="mt-5 rounded-md bg-accent px-4 py-2 text-sm font-medium text-accent-fg hover:bg-accent-hover"
					onclick={() => void prepareTranscode()}
				>
					<RotateCcw class="mr-1.5 inline size-3.5" /> Retry preparation
				</button>
			</div>
		</div>
	{/if}

	<!-- Buffering -->
	{#if waiting && !error && !preparingTranscode}
		<div class="pointer-events-none absolute inset-0 flex items-center justify-center">
			<Spinner class="size-10 text-white/80" label="Buffering" />
		</div>
	{/if}

	<!-- Error -->
	{#if error}
		<div class="absolute inset-0 flex items-center justify-center bg-black/80 px-6" role="alert">
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
	{#if transcodeReady && (ended || (!playing && !error && !waiting && currentTime === 0)) && controlsVisible}
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
		<div class="min-w-0">
			<p class="truncate text-sm font-medium text-white/90">{title}</p>
			{#if plan.mode === 'transcode' && transcodeReasons.length > 0}
				<p class="truncate text-xs text-white/55">
					Transcoding: {transcodeReasons.join(' · ')}{delivery === 'hls' ? ' · HLS' : ''}
				</p>
			{/if}
		</div>
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
				max={seekMax}
				step={0.1}
				onpointerdown={() => (seekEngaged = true)}
				onValueChange={(v) => {
					/*
					 * bits-ui also emits this callback when it re-snaps the
					 * controlled `value` prop onto the step grid — an echo of
					 * the position we passed in, not a user gesture. Only a
					 * change that departs from playback starts a scrub;
					 * latching it on the echo leaves `onValueCommit` forever
					 * un-called, which used to freeze the readout and keep the
					 * controls on screen.
					 */
					if (!scrubbing && Math.abs(v - currentTime) <= 0.06) return;
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

			<div class="ml-2 flex items-center gap-1.5">
				<IconButton label={muted ? 'Unmute' : 'Mute'} class="text-white/85 hover:text-white" onclick={toggleMute}>
					<VolumeIcon class="size-5" />
				</IconButton>
				<div class="hidden w-24 md:block">
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
