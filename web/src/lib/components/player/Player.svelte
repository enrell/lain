<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { Slider } from 'bits-ui';
	import Hls from 'hls.js';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Settings from '@lucide/svelte/icons/settings';
	import X from '@lucide/svelte/icons/x';
	import Maximize from '@lucide/svelte/icons/maximize';
	import Minimize from '@lucide/svelte/icons/minimize';
	import Pause from '@lucide/svelte/icons/pause';
	import Play from '@lucide/svelte/icons/play';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import SkipBack from '@lucide/svelte/icons/rotate-ccw';
	import SkipForward from '@lucide/svelte/icons/rotate-cw';
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
	import { VideoRenderer } from '$lib/player/webgpu/engine';
	import { PASSTHROUGH_PASS } from '$lib/player/webgpu/shaders';
	import { ANIME4K_DOG_X2 } from '$lib/player/webgpu/packs/anime4k';

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
	// How long a direct play has to prove a decoded picture (D-058).
	const FRAME_PROOF_MS = 3200;

	let container = $state<HTMLDivElement | null>(null);
	let video = $state<HTMLVideoElement | null>(null);
	let renderCanvas = $state<HTMLCanvasElement | null>(null);
	let gpuRenderer: VideoRenderer | null = null;
	let rendererActive = $state(false);
	let rendererGeneration = 0;
	let selectedEffect = $state<'off' | 'anime4k-dog-x2'>('off');
	const rendererPasses = $derived(
		selectedEffect === 'anime4k-dog-x2' ? ANIME4K_DOG_X2 : [PASSTHROUGH_PASS]
	);

	let playing = $state(false);
	let waiting = $state(false);
	let ended = $state(false);
	let currentTime = $state(0);
	let duration = $state(0);
	// The buffered range in the source timeline, so a session that starts
	// partway in paints its ready region where it actually is.
	let bufferedStart = $state(0);
	let bufferedEnd = $state(0);
	let volume = $state(prefs.getNumber('player.volume', 1));
	let muted = $state(prefs.getBool('player.muted', false));
	let rate = $state(prefs.getNumber('player.rate', 1));
	let fullscreen = $state(false);
	let fullscreenNotice = $state<string | null>(null);
	let controlsVisible = $state(true);
	let settingsOpen = $state(false);
	let settingsButton = $state<HTMLButtonElement | null>(null);
	$effect(() => {
		if (settingsOpen) void tick().then(() => chromeTailEl?.querySelector<HTMLSelectElement>('select')?.focus());
	});
	// The chrome's own elements. The reveal zones are measured from these, so
	// the layout decides where the pointer may wake the chrome instead of a
	// hardcoded height that another palette, viewport or font would break.
	let headerEl = $state<HTMLElement | null>(null);
	let chromeTailEl = $state<HTMLElement | null>(null);
	let chromeFootEl = $state<HTMLElement | null>(null);
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
	// seekTarget is the position a rebuild is preparing from, so the overlay
	// can say "seeking" instead of telling the first-play story.
	let seekTarget = $state(0);
	let hls: Hls | null = null;
	// Set on unmount so a late status/attach continuation cannot create an
	// Hls instance after destroyHLS() has already run.
	let destroyed = false;
	// A direct plan is a claim until a picture exists. A browser can open
	// the container, report no error, advance the clock and decode nothing
	// (measured: HEVC in Matroska paints zero frames at a 0x0 video size),
	// so a direct play is on probation: without a picture in time the
	// player switches to the prepared path instead of leaving a black
	// rectangle (D-058). forcedTranscode is that switch, and `mode` — never
	// `plan.mode` — is what the rest of this component asks.
	let forcedTranscode = $state(false);
	let directFallbackNote = $state(false);
	let frameCheck: ReturnType<typeof setTimeout> | null = null;
	const mode = $derived(forcedTranscode ? 'transcode' : plan.mode);

	const audioTracks = $derived((plan.streams ?? []).filter((s) => s.type === 'audio'));
	const subtitleTracks = $derived(
		(plan.streams ?? []).filter((s) => s.type === 'subtitle' && s.convertible)
	);
	// Transcode sessions serve the sidecar the session prepared; direct
	// play asks the server to extract the selected track on the fly.
	const subtitleSrc = $derived(
		mode === 'transcode' && transcodeReady && hasSubtitle && transcodeSession
			? api.playback.subtitleUrl(item.id, session.token, transcodeSession)
			: mode !== 'transcode' && selectedSubtitle !== ''
				? api.playback.streamSubtitleUrl(item.id, session.token, Number(selectedSubtitle))
				: undefined
	);

	let hideTimer: ReturnType<typeof setTimeout> | null = null;
	let resumeTimer: ReturnType<typeof setTimeout> | null = null;
	let resumeApplied = false;
	let transcodeAbort: AbortController | null = null;

	const streamSrc = $derived(
		mode === 'transcode' && delivery === 'hls'
			? undefined
			: mode === 'transcode' && !transcodeReady
				? undefined
				: mode === 'transcode'
					? api.playback.transcodeUrl(item.id, session.token, transcodeSession)
					: api.playback.streamUrl(item.id, session.token)
	);
	const title = $derived(item.title);
	// Only a file with a video track can prove itself with a picture:
	// audio-only direct play has no frame to wait for.
	const expectsVideo = $derived((plan.streams ?? []).some((s) => s.type === 'video'));

	/*
	 * How long the media really is, from the plan's probe (D-057). 0 means
	 * the server did not probe, so the element's own duration is all we
	 * have. This is what makes the seek bar span the episode under HLS: an
	 * EVENT playlist lists only the segments ffmpeg has written, so hls.js
	 * sets the MediaSource duration to the produced edge.
	 */
	const sourceDuration = $derived(
		plan.duration_sec && plan.duration_sec > 0 ? plan.duration_sec : 0
	);
	const totalDuration = $derived(sourceDuration > 0 ? sourceDuration : duration);
	// What the current session still has to produce, in its own timeline.
	const sessionDuration = $derived(
		sourceDuration > 0 ? Math.max(0, sourceDuration - baseOffset) : 0
	);

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
		if (mode === 'transcode') {
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
			startFrameProof();
		}
		return () => {
			window.removeEventListener('pagehide', onPageHide);
			document.removeEventListener('visibilitychange', onPageHide);
		};
	});

	onDestroy(() => {
		destroyed = true;
		stopRenderer();
		persistNow();
		if (hideTimer) clearTimeout(hideTimer);
		if (resumeTimer) clearTimeout(resumeTimer);
		if (frameCheck) clearTimeout(frameCheck);
		transcodeAbort?.abort();
		destroyHLS();
	});

	/* ---------------------------------------------------------------- */
	/* optional WebGPU presentation layer                                */
	/* ---------------------------------------------------------------- */

	function stopRenderer(): void {
		rendererGeneration++;
		gpuRenderer?.stop();
		gpuRenderer = null;
		rendererActive = false;
	}

	// The browser media engine remains authoritative. WebGPU starts only
	// after playback has a usable source and yields the picture only after
	// its first successful GPU submission. Native text tracks temporarily
	// return presentation to <video>, whose caption renderer owns them.
	$effect(() => {
		const el = video;
		const canvas = renderCanvas;
		const ready = transcodeReady;
		const nativeSubtitle = subtitleSrc;
		const passes = rendererPasses;
		if (!el || !canvas || !ready || nativeSubtitle) {
			stopRenderer();
			return;
		}

		const generation = ++rendererGeneration;
		let cancelled = false;
		void VideoRenderer.create({
			video: el,
			canvas,
			passes,
			onActiveChange: (active) => {
				if (!cancelled && generation === rendererGeneration) rendererActive = active;
			},
			onFailure: (reason) => {
				if (cancelled || generation !== rendererGeneration) return;
				gpuRenderer = null;
				rendererActive = false;
				if (import.meta.env.DEV) console.warn('[lain-player] WebGPU renderer disabled', reason);
			}
		}).then((result) => {
			if (cancelled || generation !== rendererGeneration) {
				result.renderer?.stop();
				return;
			}
			gpuRenderer = result.renderer;
			if (!result.renderer && import.meta.env.DEV) {
				console.info('[lain-player] native video renderer', result.reason);
			}
		});

		return () => {
			cancelled = true;
			if (generation === rendererGeneration) stopRenderer();
		};
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
			// hls.js would otherwise set the MediaSource duration to the
			// playlist edge — which is why the bar used to stop at whatever
			// ffmpeg had encoded. Handing it the real length is the supported
			// way (MediaOverrides.duration); endOfStream stays hls.js's call,
			// driven by the playlist's ENDLIST, so this can never end the
			// stream early.
			if (sessionDuration > 0) {
				hls.attachMedia({ media: el, overrides: { duration: sessionDuration } });
			} else {
				hls.attachMedia(el);
			}
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
		if (mode === 'transcode') void changeTracks();
	}

	async function changeTracks(): Promise<void> {
		// Rebuild the session from the current display position so the swap
		// keeps the viewer's place (the new session's timeline restarts at 0).
		await rebuildAt(currentTime);
	}

	// rebuildAt starts a fresh session whose timeline begins at position,
	// which is how a seek past the produced edge, a quality change and a
	// track change all keep the viewer where they were.
	async function rebuildAt(position: number): Promise<void> {
		if (mode !== 'transcode') return;
		const oldSession = transcodeSession;
		destroyHLS();
		transcodeReady = false;
		resumeApplied = true;
		baseOffset = position;
		if (oldSession) void api.playback.cancelTranscode(item.id, oldSession).catch(() => undefined);
		try {
			await prepareTranscode({
				audio_stream: selectedAudio === '' ? undefined : Number(selectedAudio),
				subtitle_stream: selectedSubtitle === '' ? undefined : Number(selectedSubtitle),
				quality: selectedQuality || undefined
			});
		} finally {
			seekTarget = 0;
		}
		if (video && transcodeReady) {
			video.currentTime = 0;
			currentTime = position;
			void video.play().catch(() => undefined);
		}
	}

	/* ---------------------------------------------------------------- */
	/* first-frame verification (D-058)                                  */
	/* ---------------------------------------------------------------- */

	// decodedPicture is the only honest evidence that a direct play works:
	// `loadedmetadata` and a resolved `play()` both succeed on a file the
	// browser cannot decode. totalVideoFrames is Chromium's; where it is
	// absent the painted size still has to be non-zero.
	function decodedPicture(el: HTMLVideoElement): boolean {
		const frames = (el as HTMLVideoElement & { totalVideoFrames?: number }).totalVideoFrames;
		return el.videoWidth > 0 && (frames === undefined || frames > 0);
	}

	function startFrameProof(): void {
		if (mode !== 'direct' || !expectsVideo) return;
		if (frameCheck) clearTimeout(frameCheck);
		frameCheck = setTimeout(() => {
			frameCheck = null;
			if (video && decodedPicture(video)) return;
			void fallbackToTranscode();
		}, FRAME_PROOF_MS);
	}

	function confirmFrameProof(): void {
		if (!frameCheck) return;
		if (video && decodedPicture(video)) {
			clearTimeout(frameCheck);
			frameCheck = null;
		}
	}

	// fallbackToTranscode is the honesty layer under the capability claim:
	// the browser said it could decode this file and did not. The viewer
	// keeps their place and gets the prepared version, with the reason said
	// out loud instead of a black frame.
	async function fallbackToTranscode(): Promise<void> {
		if (forcedTranscode) return;
		forcedTranscode = true;
		directFallbackNote = true;
		destroyHLS();
		const el = video;
		const at = el && el.currentTime > 1 ? el.currentTime : resumePosition();
		el?.pause();
		baseOffset = at;
		resumeApplied = true;
		await prepareTranscode();
		if (destroyed || !transcodeReady || !video) return;
		video.currentTime = 0;
		currentTime = at;
		void video.play().catch(() => undefined);
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
		confirmFrameProof();
		currentTime = el.currentTime + baseOffset;
		if (el.buffered.length > 0) {
			bufferedStart = el.buffered.start(0) + baseOffset;
			bufferedEnd = el.buffered.end(el.buffered.length - 1) + baseOffset;
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
		confirmFrameProof();
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
		// A direct play that fails to decode is the capability claim being
		// wrong; the honest answer is the prepared path, not an error the
		// viewer cannot act on (D-058).
		if (
			mode === 'direct' &&
			(code === MediaError.MEDIA_ERR_DECODE || code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED)
		) {
			void fallbackToTranscode();
			return;
		}
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
		if (mode === 'transcode' && delivery === 'hls') {
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

	/*
	 * The chrome fades in place (D-064). It is never unmounted: an unmount
	 * reflows the picture and moves the very rows the pointer has to find, so
	 * the reveal zone would slide out from under the cursor trying to wake it.
	 * Opacity keeps the geometry exact — which is what keeps D-061's no-reflow
	 * premise true — and `pointer-events-none` keeps an invisible row from
	 * swallowing a click meant for the picture.
	 */
	const REVEAL_MARGIN_PX = 56;

	const chromeClass = $derived(
		[
			'transition-opacity duration-200',
			controlsVisible ? 'opacity-100' : 'pointer-events-none opacity-0'
		].join(' ')
	);

	// The pointer wakes the chrome only where the chrome lives: a move over the
	// middle of the picture is watching, not a request for controls.
	function pointerNearChrome(event: PointerEvent): boolean {
		for (const el of [headerEl, chromeTailEl, chromeFootEl]) {
			if (!el) continue;
			const box = el.getBoundingClientRect();
			if (
				event.clientY >= box.top - REVEAL_MARGIN_PX &&
				event.clientY <= box.bottom + REVEAL_MARGIN_PX
			) {
				return true;
			}
		}
		return false;
	}

	function onStagePointerMove(event: PointerEvent): void {
		// While the chrome is up, any movement re-arms the hide: a viewer moving
		// the pointer across the picture must not watch it fade under their hand.
		if (!controlsVisible && !pointerNearChrome(event)) return;
		revealControls();
	}

	// Keyboard focus lands on chrome that is only faded, so focus has to reveal
	// it and hold it there: the seek bar stays reachable without a pointer, and
	// a fade must not pull the button out from under a keyboard user (D-061).
	// Only keyboard focus counts: a mouse click leaves focus on the button it
	// hit, and pinning on that would mean the deck never fades again after the
	// first click on it — which is exactly what the fullscreen button did.
	function chromeFocused(): boolean {
		const active = document.activeElement;
		if (!(active instanceof HTMLElement)) return false;
		if (!active.matches(':focus-visible')) return false;
		return headerEl?.contains(active) === true || chromeFootEl?.contains(active) === true;
	}

	// States the chrome must not fade out of: a wait that owns an overlay, a
	// failure the viewer has to act on, a resume banner, a drag in progress, or
	// focus inside the chrome itself.
	function chromePinned(): boolean {
		return settingsOpen || scrubbing || !!error || preparingTranscode || showResume || chromeFocused();
	}

	function scheduleHide(): void {
		if (hideTimer) clearTimeout(hideTimer);
		if (!playing || chromePinned()) {
			controlsVisible = true;
			return;
		}
		hideTimer = setTimeout(() => {
			hideTimer = null;
			if (playing && !chromePinned()) controlsVisible = false;
		}, HIDE_DELAY_MS);
	}

	function revealControls(): void {
		controlsVisible = true;
		scheduleHide();
	}

	// Escape puts the chrome away without leaving the episode (D-066). Focus
	// inside the chrome still pins it: hiding it would take the focused control
	// off screen with the keyboard still on it.
	function hideChrome(): void {
		if (hideTimer) clearTimeout(hideTimer);
		hideTimer = null;
		if (chromePinned()) return;
		controlsVisible = false;
	}

	/*
	 * A pin that clears has to hand the chrome back to the idle timer. The
	 * resume banner owns its own clock and can outlast `onPlay`, and a scrub
	 * can end without another pointer event: without this effect the early
	 * return above arms nothing and the chrome stays on screen for the rest of
	 * the episode, which is the very report this slice came from. Reading the
	 * pin's inputs here is what makes the release reactive; `controlsVisible`
	 * is only written, so the effect cannot re-trigger itself.
	 */
	$effect(() => {
		if (!playing) return;
		if (chromePinned()) {
			if (hideTimer) clearTimeout(hideTimer);
			hideTimer = null;
			controlsVisible = true;
			return;
		}
		scheduleHide();
	});

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

	/*
	 * A seek inside what the session has already produced is a plain element
	 * seek: hls.js holds the segment and playback continues without a
	 * hiccup. Past that, waiting for the encoder to reach the target would
	 * take as long as watching it, so the session is rebuilt at the target
	 * instead — the contract's start_sec, the same machinery a resume and a
	 * track change already use.
	 */
	const SEEK_PRODUCED_GRACE_SEC = 3;

	/*
	 * A session only holds the range it was started for: it begins at
	 * baseOffset and has produced as far as its playlist reaches. A target
	 * outside that range has nothing to fetch, so it needs a new session
	 * rather than an element seek the browser would clamp.
	 *
	 * hls.js owns the parsed playlist, so the produced edge is read from it
	 * on demand instead of mirrored in component state: no subscription to
	 * keep in sync, and a destroyed instance simply reports nothing.
	 */
	function producedEdge(): number {
		const edge = hls?.latestLevelDetails?.edge;
		return edge === undefined ? 0 : edge + baseOffset;
	}

	function needsRebuild(target: number): boolean {
		if (mode !== 'transcode' || delivery !== 'hls' || sourceDuration <= 0) return false;
		if (target < baseOffset - SEEK_PRODUCED_GRACE_SEC) return true;
		const produced = producedEdge();
		return produced > 0 && target > produced + SEEK_PRODUCED_GRACE_SEC;
	}

	function seekBy(seconds: number): void {
		seekTo(currentTime + seconds);
	}

	function seekTo(seconds: number): void {
		const el = video;
		if (!el) return;
		// The authoritative ceiling is the media's own length; the element's
		// duration is the fallback when the server could not probe.
		const ceiling = totalDuration > 0 ? totalDuration : el.duration + baseOffset;
		const target = Math.max(0, Math.min(Number.isFinite(ceiling) ? ceiling : seconds, seconds));
		revealControls();
		if (needsRebuild(target)) {
			seekTarget = target;
			void rebuildAt(target);
			return;
		}
		el.currentTime = Math.max(0, target - baseOffset);
		currentTime = target;
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

	/*
	 * Fullscreen is a request, not a guarantee: a browser can refuse it (a
	 * permission policy, a container that forbids it) or not offer the API at
	 * all. A refusal is reported in the player instead of swallowed (D-011),
	 * and playback continues windowed.
	 */
	async function toggleFullscreen(): Promise<void> {
		const el = container;
		if (!el) return;
		const request = el.requestFullscreen?.bind(el);
		try {
			if (document.fullscreenElement) {
				await document.exitFullscreen();
			} else if (request) {
				await request();
			} else {
				fullscreenNotice = 'Fullscreen is not available in this browser.';
				return;
			}
			fullscreenNotice = null;
		} catch {
			fullscreenNotice = 'Fullscreen is not available in this browser.';
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

	/*
	 * Which keys still belong to the player when a control has focus (D-066).
	 * A focused button or link keeps the two keys that activate it — Space and
	 * Enter — because that is what those keys mean on a control, and a viewer
	 * who clicks fullscreen must not lose the keyboard afterwards. The seek bar
	 * is the one slider the player owns, so its arrows seek by five seconds
	 * instead of nudging a tenth-of-a-second step grid; every other slider
	 * keeps its own keys. Text entry and dropdowns keep everything.
	 */
	function shortcutsAllowed(event: KeyboardEvent): boolean {
		if (event.metaKey || event.ctrlKey || event.altKey) return false;
		const target = event.target;
		if (isTypingTarget(target)) return false;
		if (target instanceof HTMLElement) {
			const tag = target.tagName;
			const activationKey = event.key === ' ' || event.key === 'Enter';
			if ((tag === 'BUTTON' || tag === 'A') && activationKey) return false;
			const slider = target.getAttribute('role') === 'slider';
			if (slider && !target.closest('[data-player-seek]')) return false;
		}
		return true;
	}

	/*
	 * The player listens on the capture phase, so a key it owns is claimed
	 * before the control under the focus acts on it: the slider moves its own
	 * value on an arrow key without asking whether the event was already
	 * handled, and the player's five-second seek would land on top of that
	 * step.
	 */
	function consume(event: KeyboardEvent): void {
		event.preventDefault();
		event.stopPropagation();
	}

	/*
	 * Escape dismisses what the player is showing — the settings panel, then a
	 * fullscreen notice, then the chrome — and never leaves the episode: Back is
	 * the way out, and a viewer reaching for Escape must not lose playback.
	 */
	function onEscape(event: KeyboardEvent): void {
		if (settingsOpen) {
			consume(event);
			settingsOpen = false;
			settingsButton?.focus();
			return;
		}
		if (fullscreenNotice) {
			consume(event);
			fullscreenNotice = null;
			return;
		}
		if (!controlsVisible || !shortcutsAllowed(event)) return;
		consume(event);
		hideChrome();
	}

	function onKeydown(event: KeyboardEvent): void {
		if (event.key === 'Escape') {
			onEscape(event);
			return;
		}
		if (!shortcutsAllowed(event)) return;
		switch (event.key) {
			case ' ':
			case 'k':
			case 'K':
				consume(event);
				togglePlay();
				revealControls();
				break;
			case 'ArrowLeft':
				consume(event);
				seekBy(-5);
				break;
			case 'ArrowRight':
				consume(event);
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
				consume(event);
				setVolume(volume + 0.05);
				revealControls();
				break;
			case 'ArrowDown':
				consume(event);
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
		}
	}

	const VolumeIcon = $derived(muted || volume === 0 ? VolumeX : volume < 0.5 ? Volume1 : Volume2);
	// A zero max collapses the slider's step grid to a single step, so every
	// playback position would be snapped away and echoed back as a change.
	// The bar spans the media, not what the encoder has reached so far.
	const seekMax = $derived(totalDuration > 0 ? totalDuration : 1);
	const bufferedLeft = $derived(
		seekMax > 0 ? Math.min(100, Math.max(0, (bufferedStart / seekMax) * 100)) : 0
	);
	const bufferedWidth = $derived(
		seekMax > 0
			? Math.max(0, Math.min(100 - bufferedLeft, ((bufferedEnd - bufferedStart) / seekMax) * 100))
			: 0
	);
	// Preparation is the one wait with no media element reporting for it, so
	// the overlay counts from the server's own start time and shows the
	// fraction when the server has one (D-038/D-039).
	const prepareElapsed = $derived(elapsedSince(prepareStartedAt, nowMs));
	const preparePercent = $derived(Math.round(prepareProgress * 100));

	// Optional playback information uses the actual post-fallback session.
	const telemetry = $derived(
		mode === 'transcode'
			? [
					delivery === 'hls' ? 'HLS' : delivery === 'progressive' ? 'MP4' : 'Session',
					selectedQuality || 'Auto'
				].join(' · ')
			: 'Direct play'
	);
	// Technical details stay available without competing with the picture.
	const sessionNote = $derived(
		[
			mode === 'transcode' && transcodeReasons.length > 0
				? `Transcoding · ${transcodeReasons.join(' · ')}`
				: '',
			mode === 'transcode' && delivery
				? delivery === 'hls'
					? 'HLS delivery'
					: 'Progressive MP4'
				: ''
		]
			.filter(Boolean)
			.join('  ·  ')
	);
</script>

<svelte:window
	onkeydowncapture={onKeydown}
	onpointerup={() => (seekEngaged = false)}
	onpointercancel={() => (seekEngaged = false)}
/>

<div
	bind:this={container}
	class={[
		// The picture is the whole region and the chrome floats over it, so the
		// chrome rows are the only children in the flow, pinned to the bottom.
		'relative flex h-full w-full flex-col justify-end overflow-hidden bg-background',
		// An idle pointer over a full-bleed picture is an arrow with nothing to
		// point at; it comes back with the chrome.
		controlsVisible ? '' : 'cursor-none'
	].join(' ')}
	role="region"
	aria-label={`Player — ${title}`}
	onpointermove={onStagePointerMove}
	onpointerdown={(event) => {
		if (settingsOpen && event.target instanceof Node && !chromeTailEl?.contains(event.target) && !settingsButton?.contains(event.target)) settingsOpen = false;
		revealControls();
	}}
	onfocusin={revealControls}
>
	<!-- Palette-owned chrome, floating over the picture (D-065/D-066): the bar over the top
	     edge and the deck over the bottom one, each on the palette's own
	     background at 90% with a blur — the idiom the app's own nav already uses,
	     and the opacity that keeps `muted` text above the 4.5:1 floor even when
	     the frame behind it is white (D-020/D-037). -->
	<header
		bind:this={headerEl}
		class={[
			'absolute inset-x-0 top-0 z-20 flex h-16 items-center gap-4 bg-background/90 px-4 backdrop-blur-xl sm:px-6',
			chromeClass
		].join(' ')}
	>
		<button
			type="button"
			class="inline-flex min-h-11 shrink-0 items-center gap-2 rounded-md px-2 text-sm text-foreground transition-colors hover:bg-surface-hover"
			onclick={() => void goto(`/item/${item.id}`)}
		>
			<ArrowLeft class="size-5" aria-hidden="true" /> Back
		</button>
		<p
			class="min-w-0 truncate text-base font-medium text-foreground sm:text-lg"
		>
			{title}
		</p>
	</header>

	<div class="absolute inset-0 letterbox">
		<video
			bind:this={video}
			src={streamSrc}
			class={[
				'absolute inset-0 size-full object-contain transition-opacity duration-100',
				rendererActive ? 'opacity-0' : 'opacity-100'
			].join(' ')}
			preload="metadata"
			playsinline
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
		<canvas
			bind:this={renderCanvas}
			class={[
				'pointer-events-none absolute inset-0 size-full object-contain transition-opacity duration-100',
				rendererActive ? 'opacity-100' : 'opacity-0'
			].join(' ')}
			aria-hidden="true"
		></canvas>

	{#if preparingTranscode}
		<div class="absolute inset-0 flex flex-col items-center justify-center gap-3 bg-background/85 px-6 text-center">
			<Spinner class="size-10 text-muted" label="Preparing playback" />
			<div class="max-w-md">
				<p class="text-sm text-foreground">
					{seekTarget > 0
						? `Seeking to ${formatTime(seekTarget)}…`
						: 'Preparing a browser-compatible version…'}
				</p>
				<p class="mt-2 font-mono text-xs uppercase tracking-[0.14em] tabular-nums text-muted">
					{formatTime(prepareElapsed)} elapsed
				</p>
				{#if prepareProgress > 0}
					<div
						class="mx-auto mt-4 h-[3px] w-56 overflow-hidden bg-line/30"
						role="progressbar"
						aria-label="Preparation progress"
						aria-valuemin="0"
						aria-valuemax="100"
						aria-valuenow={preparePercent}
					>
						<div class="h-full bg-accent" style={`width:${preparePercent}%`}></div>
					</div>
					<p class="mt-1.5 font-mono text-[11px] uppercase tracking-[0.14em] text-muted">
						{preparePercent}% prepared
					</p>
				{/if}
				{#if seekTarget > 0}
					<p class="mt-4 text-xs leading-relaxed text-muted">
						The server starts producing this session from that point, so playback resumes shortly.
					</p>
				{:else}
					<p class="mt-4 text-xs leading-relaxed text-muted">
						The first play prepares a browser-compatible copy of this file. Sources like AV1 or
						HEVC need a full re-encode and can take several minutes.
					</p>
				{/if}
				<p class="mt-2 text-xs leading-relaxed text-muted">
					You can leave this page — preparation continues on the server.
				</p>
				<button
					class="mt-5 inline-flex items-center gap-1.5 border border-line/40 px-4 py-2 font-mono text-[11px] uppercase tracking-[0.14em] text-foreground hover:bg-surface-hover"
					onclick={() => void goto(`/item/${item.id}`)}
				>
					<ArrowLeft class="size-3.5" aria-hidden="true" /> Back to details
				</button>
			</div>
		</div>
	{:else if transcodeError}
		<div class="absolute inset-0 flex items-center justify-center bg-background/85 px-6" role="alert">
			<div class="max-w-md text-center">
				<TriangleAlert class="mx-auto size-8 text-warning" />
				<p class="mt-3 text-sm text-foreground">{transcodeError}</p>
				<button
					class="mt-5 inline-flex items-center gap-1.5 bg-accent px-4 py-2 font-mono text-[11px] uppercase tracking-[0.14em] text-accent-fg hover:bg-accent-hover"
					onclick={() => void prepareTranscode()}
				>
					<RotateCcw class="size-3.5" aria-hidden="true" /> Retry preparation
				</button>
			</div>
		</div>
	{/if}

	<!-- Buffering -->
	{#if waiting && !error && !preparingTranscode}
		<div class="pointer-events-none absolute inset-0 flex items-center justify-center">
			<Spinner class="size-10 text-muted" label="Buffering" />
		</div>
	{/if}

	<!-- Error -->
	{#if error}
		<div class="absolute inset-0 flex items-center justify-center bg-background/85 px-6" role="alert">
			<div class="max-w-md text-center">
				<TriangleAlert class="mx-auto size-8 text-warning" />
				<p class="mt-3 text-sm text-foreground">{error}</p>
				<div class="mt-5 flex justify-center gap-2">
					<button
						class="inline-flex items-center gap-1.5 bg-accent px-4 py-2 font-mono text-[11px] uppercase tracking-[0.14em] text-accent-fg hover:bg-accent-hover"
						onclick={retryPlayback}
					>
						<RotateCcw class="size-3.5" aria-hidden="true" /> Retry
					</button>
					<a
						href={`/item/${item.id}`}
						class="inline-flex items-center border border-line/40 px-4 py-2 font-mono text-[11px] uppercase tracking-[0.14em] text-foreground hover:bg-surface-hover"
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
				class="flex size-20 items-center justify-center rounded-full border border-foreground/25 bg-background/60 text-foreground backdrop-blur transition-transform duration-150 hover:scale-105"
			>
				<Play class="size-9 translate-x-0.5" />
			</span>
		</button>
	{/if}

	<!-- Resume banner -->
	{#if showResume && resumedFrom > 0}
		<div
			class="absolute left-4 top-20 rounded-md border border-line/40 bg-background/95 px-4 py-3 text-sm text-foreground"
		>
			Resuming from {formatTime(resumedFrom)}
			<button class="ml-3 text-accent hover:text-accent-hover" onclick={skipResumeToStart}>
				Start over
			</button>
		</div>
	{/if}

	<!-- A refused fullscreen request is a failure the viewer asked for: it is
	     reported where they are looking, and playback keeps going windowed. -->
	{#if fullscreenNotice}
		<div
			data-fullscreen-notice
			role="status"
			class="absolute right-4 top-20 z-30 flex max-w-sm items-start gap-2 rounded-md border border-line/40 bg-background/95 px-4 py-2.5 text-sm text-foreground shadow-lg backdrop-blur-xl sm:right-6"
		>
			<span>{fullscreenNotice}</span>
			<IconButton
				label="Dismiss fullscreen notice"
				onclick={() => (fullscreenNotice = null)}
			>
				<X class="size-4" />
			</IconButton>
		</div>
	{/if}

	</div>

	<!-- Keep settings inside the player so they also work in fullscreen. -->
	{#if settingsOpen}
		<div
			bind:this={chromeTailEl}
			id="player-settings"
			role="region"
			aria-label="Playback settings"
			class="player-settings absolute bottom-28 right-3 z-30 max-h-[calc(100%-12rem)] w-[min(22rem,calc(100%-1.5rem))] overflow-y-auto rounded-lg border border-line/60 bg-background p-4 text-foreground shadow-xl sm:right-6"
		>
			<div class="mb-3 flex items-center justify-between">
				<h2 class="text-base font-semibold">Playback settings</h2>
				<IconButton label="Close settings" onclick={() => { settingsOpen = false; settingsButton?.focus(); }}><X class="size-5" /></IconButton>
			</div>
			<label class="chrome-chip">
				Speed
				<select class="chrome-select" value={rate} onchange={(event) => setRate(Number(event.currentTarget.value))} aria-label="Playback speed">
					{#each RATES as option (option)}<option value={option}>{option === 1 ? 'Normal' : `${option}×`}</option>{/each}
				</select>
			</label>
			{#if rendererActive}
				<label class="chrome-chip">
					Effects
					<select class="chrome-select" bind:value={selectedEffect} aria-label="Video effects">
						<option value="off">Off</option>
						<option value="anime4k-dog-x2">Anime4K DoG ×2</option>
					</select>
				</label>
			{/if}
			{#if mode === 'transcode' && (playbackOptions?.qualities.length ?? 0) > 0}
				<label class="chrome-chip">
					Quality
					<select
						class="chrome-select"
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
			{#if mode === 'transcode' && audioTracks.length > 1}
				<label class="chrome-chip">
					Audio
					<select
						class="chrome-select"
						bind:value={selectedAudio}
						onchange={() => void changeTracks()}
						aria-label="Audio track"
					>
						<option value="">Default</option>
						{#each audioTracks as track (track.index)}
							<option value={String(track.index)}>
								{track.language || track.title || `Track ${track.index}`}
							</option>
						{/each}
					</select>
				</label>
			{/if}
			{#if subtitleTracks.length > 0}
				<label class="chrome-chip">
					Subtitles
					<select
						class="chrome-select"
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
			<details class="mt-4 border-t border-line/40 pt-3 text-xs leading-relaxed text-muted">
				<summary class="cursor-pointer py-2 text-sm">Playback information</summary>
				<p class="mt-2 font-mono">{telemetry}</p>
				{#if sessionNote}<p class="mt-2">{sessionNote}</p>{/if}
			</details>
		</div>
	{/if}

	{#if directFallbackNote}
		<p
			class={[
				'relative z-20 bg-background/90 px-4 pt-2 text-xs leading-relaxed text-muted backdrop-blur-xl',
				chromeClass
			].join(' ')}
		>
			Preparing this video for your browser.
		</p>
	{/if}

	<!-- Transport deck: it spans the whole media (D-057), it floats over the
	     picture's bottom edge (D-065), and it fades with the rest of the chrome
	     (D-064). -->
	<footer
		bind:this={chromeFootEl}
		class={[
			'relative z-20 bg-background/90 px-3 pb-3 pt-2 backdrop-blur-xl sm:px-6 sm:pb-4',
			chromeClass
		].join(' ')}
	>
		<!-- Seek -->
		<div class="group/seek flex items-center gap-3">
			<Slider.Root
				type="single"
				value={scrubbing ? scrubValue : currentTime}
				max={seekMax}
				step={0.1}
				data-player-seek
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
				<span class="relative h-[3px] w-full grow overflow-hidden bg-line/25">
					<!-- The ready range sits where it actually is: a resumed or
					     re-seeked session begins partway into the episode. -->
					<span
						class="absolute inset-y-0 bg-foreground/25"
						style={`left:${bufferedLeft}%;width:${bufferedWidth}%`}
					></span>
					<Slider.Range class="absolute h-full bg-accent" />
				</span>
				<Slider.Thumb
					index={0}
					aria-label="Seek"
					class="block size-3.5 rounded-full bg-accent transition-transform duration-150 hover:scale-125 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
				/>
			</Slider.Root>
			<span class="shrink-0 text-xs tabular-nums text-foreground sm:text-sm">
				{formatTime(scrubbing ? scrubValue : currentTime)} <span class="text-muted">/ {formatTime(totalDuration)}</span>
			</span>
		</div>

		<!-- Transport -->
		<div class="mt-1 flex items-center gap-1.5">
			<IconButton label={playing ? 'Pause' : 'Play'} class="size-11 bg-accent text-accent-fg hover:bg-accent-hover hover:text-accent-fg" onclick={togglePlay}>
				{#if playing}<Pause class="size-6" />{:else}<Play class="size-6" />{/if}
			</IconButton>
			<IconButton label="Back 10 seconds" class="relative size-11" onclick={() => seekBy(-10)}>
				<SkipBack class="size-6" /><span class="absolute text-[9px] font-semibold">10</span>
			</IconButton>
			<IconButton label="Forward 10 seconds" class="relative size-11" onclick={() => seekBy(10)}>
				<SkipForward class="size-6" /><span class="absolute text-[9px] font-semibold">10</span>
			</IconButton>

			<div class="flex items-center gap-1">
				<IconButton label={muted ? 'Unmute' : 'Mute'} onclick={toggleMute}>
					<VolumeIcon class="size-5" />
				</IconButton>
				<div class="hidden w-20 md:block">
					<Slider.Root
						type="single"
						value={muted ? 0 : volume}
						max={1}
						step={0.02}
						onValueChange={setVolume}
						class="relative flex h-6 touch-none select-none items-center"
					>
						<span class="relative h-[3px] w-full grow overflow-hidden bg-line/25">
							<Slider.Range class="absolute h-full bg-foreground" />
						</span>
						<Slider.Thumb
							index={0}
							aria-label="Volume"
							class="block size-3 rounded-full bg-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
						/>
					</Slider.Root>
				</div>
			</div>

			<div class="ml-auto flex items-center gap-1.5">
				<button bind:this={settingsButton} type="button" aria-label="Playback settings" aria-expanded={settingsOpen} aria-controls="player-settings" class="inline-flex size-11 items-center justify-center rounded-md text-foreground hover:bg-surface-hover" onclick={() => (settingsOpen = !settingsOpen)}>
					<Settings class="size-5" />
					<span class="sr-only">Playback settings</span>
				</button>
				<IconButton
					label={fullscreen ? 'Exit fullscreen' : 'Fullscreen'}
					onclick={() => void toggleFullscreen()}
				>
					{#if fullscreen}<Minimize class="size-5" />{:else}<Maximize class="size-5" />{/if}
				</IconButton>
			</div>
		</div>
	</footer>
</div>

<style>
	.player-settings .chrome-chip {
		display: flex;
		justify-content: space-between;
		gap: 1rem;
		padding: 0.75rem 0;
		border: 0;
		font-family: inherit;
		font-size: 0.875rem;
		letter-spacing: normal;
		text-transform: none;
		color: var(--color-foreground);
	}
	.player-settings .chrome-select {
		max-width: 65%;
		min-height: 2.25rem;
		padding: 0.25rem 0.5rem;
		border: 1px solid var(--color-line);
		border-radius: 0.375rem;
		background: var(--color-background);
	}
	@media (max-width: 420px) {
		footer > div:last-child { gap: 0; }
	}
	@media (prefers-reduced-motion: reduce) {
		header, footer { transition: none; }
	}
</style>
