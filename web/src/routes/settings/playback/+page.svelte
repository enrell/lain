<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import Activity from '@lucide/svelte/icons/activity';
	import AudioLines from '@lucide/svelte/icons/audio-lines';
	import Clapperboard from '@lucide/svelte/icons/clapperboard';
	import Cpu from '@lucide/svelte/icons/cpu';
	import Gauge from '@lucide/svelte/icons/gauge';
	import HardDrive from '@lucide/svelte/icons/hard-drive';
	import Radio from '@lucide/svelte/icons/radio';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import Save from '@lucide/svelte/icons/save';
	import Search from '@lucide/svelte/icons/search';
	import Sparkles from '@lucide/svelte/icons/sparkles';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import WandSparkles from '@lucide/svelte/icons/wand-sparkles';
	import type {
		TranscodeCapabilities,
		TranscodeQuality,
		TranscodeSettings,
		TranscodeStatus
	} from '$lib/api/types';
	import { api } from '$lib/api';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import {
		applySafeAutomaticPolicy,
		availableHardwareBackends,
		cloneTranscodeSettings,
		settingsEqual
	} from '$lib/settings/playback-policy';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let loading = $state(true);
	let saving = $state(false);
	let error = $state<string | null>(null);
	let settings = $state<TranscodeSettings | null>(null);
	let persistedSettings = $state<TranscodeSettings | null>(null);
	let capabilities = $state<TranscodeCapabilities | null>(null);
	let sessions = $state<TranscodeStatus[]>([]);
	let sessionBusy = $state<string | null>(null);
	let searchQuery = $state('');
	let refreshTimer: ReturnType<typeof setInterval> | null = null;

	const dirty = $derived(!settingsEqual(settings, persistedSettings));
	const readyHardware = $derived(availableHardwareBackends(capabilities));
	const activeSessions = $derived(
		sessions.filter((session) => session.state === 'queued' || session.state === 'running').length
	);
	const normalizedSearch = $derived(searchQuery.trim().toLowerCase());
	const hasSearchResults = $derived(
		!normalizedSearch ||
		'diagnostics capabilities ffmpeg encoder probe delivery stream hls segment timeout throttle encoding quality codec crf preset hevc av1 deinterlace hardware acceleration gpu vaapi nvenc qsv decode hdr processing tone mapping luminance audio subtitle captions downmix font bitrate performance resources storage cache queue path threads active sessions running fps'.includes(
			normalizedSearch
		)
	);
	const hardwareActive = $derived(
		!!settings && settings.hardware_acceleration !== 'none'
	);
	const customEncodingActive = $derived(
		!!settings &&
		(settings.allow_hevc ||
			settings.allow_av1 ||
			!!settings.h264_preset ||
			!!settings.h265_preset ||
			!!settings.av1_preset ||
			settings.deinterlace_double_rate)
	);
	const customResourcesActive = $derived(
		!!settings &&
		(!!settings.transcode_temp_path ||
			settings.remote_bitrate_limit_kbps > 0 ||
			!!settings.ffmpeg_path ||
			!!settings.ffprobe_path ||
			settings.thread_count > 0)
	);

	const presets = [
		'ultrafast',
		'superfast',
		'veryfast',
		'faster',
		'fast',
		'medium',
		'slow',
		'slower',
		'veryslow'
	];
	const deliveries = [
		{ value: 'hls', label: 'HLS — playable while preparing' },
		{ value: 'progressive', label: 'Progressive MP4 — waits for the full file' }
	];
	const toneAlgorithms = [
		{ value: 'bt2390', label: 'BT.2390 (needs libplacebo/Vulkan)' },
		{ value: 'hable', label: 'Hable' },
		{ value: 'reinhard', label: 'Reinhard' },
		{ value: 'mobius', label: 'Mobius' },
		{ value: 'clip', label: 'Clip' },
		{ value: 'linear', label: 'Linear' }
	];
	const toneModes = [
		{ value: 'auto', label: 'Auto — only HDR sources' },
		{ value: 'always', label: 'Always' },
		{ value: 'never', label: 'Never (HDR playback stays unavailable)' }
	];
	const containers = [
		{ value: 'fmp4', label: 'fMP4 (CMAF) — default' },
		{ value: 'mpegts', label: 'MPEG-TS — legacy players' }
	];
	const hardwareOptions = [
		{ value: 'none', label: 'None (software)' },
		{ value: 'vaapi', label: 'VAAPI' },
		{ value: 'nvenc', label: 'NVIDIA NVENC' },
		{ value: 'qsv', label: 'Intel Quick Sync' },
		{ value: 'amf', label: 'AMD AMF' },
		{ value: 'v4l2m2m', label: 'V4L2 mem2mem' },
		{ value: 'videotoolbox', label: 'VideoToolbox (macOS)' },
		{ value: 'rkmpp', label: 'Rockchip RKMPP' }
	];
	const probedHardwareOptions = $derived(
		hardwareOptions.map((option) => ({
			...option,
			disabled:
				option.value !== 'none' &&
				capabilities !== null &&
				capabilities.hardware[option.value] !== true,
			label:
				option.value !== 'none' &&
				capabilities !== null &&
				capabilities.hardware[option.value] !== true
					? `${option.label} — unavailable`
					: option.label
		}))
	);
	const subtitleModes = [
		{ value: 'auto', label: 'Auto — extract text, burn image tracks' },
		{ value: 'extract', label: 'Extract only (reject image tracks)' },
		{ value: 'burn', label: 'Always burn in' },
		{ value: 'off', label: 'Off' }
	];
	const deinterlaceModes = [
		{ value: 'auto', label: 'Auto — deinterlace when detected' },
		{ value: 'off', label: 'Off' }
	];
	const deinterlaceMethods = [
		{ value: 'yadif', label: 'yadif' },
		{ value: 'bwdif', label: 'bwdif — better, when the build has it' }
	];
	const decodeCodecs = ['h264', 'hevc', 'mpeg2', 'vc1', 'vp8', 'vp9', 'av1'];
	const downmixAlgorithms = [
		{ value: 'none', label: 'None — ffmpeg default downmix' },
		{ value: 'nightmode', label: "lain nightmode — dialogue lift" }
	];

	// codecRows exposes the per-codec encoder settings as small
	// accessors, so the markup stays a plain list instead of three
	// near-identical blocks.
	const codecRows = [
		{
			key: 'h264',
			label: 'H.264',
			crf: () => settings?.h264_crf ?? 0,
			preset: () => settings?.h264_preset ?? '',
			setCRF: (value: number) => settings && (settings.h264_crf = value),
			setPreset: (value: string) => settings && (settings.h264_preset = value)
		},
		{
			key: 'h265',
			label: 'H.265',
			crf: () => settings?.h265_crf ?? 0,
			preset: () => settings?.h265_preset ?? '',
			setCRF: (value: number) => settings && (settings.h265_crf = value),
			setPreset: (value: string) => settings && (settings.h265_preset = value)
		},
		{
			key: 'av1',
			label: 'AV1',
			crf: () => settings?.av1_crf ?? 0,
			preset: () => settings?.av1_preset ?? '',
			setCRF: (value: number) => settings && (settings.av1_crf = value),
			setPreset: (value: string) => settings && (settings.av1_preset = value)
		}
	];

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			const response = await api.transcodeAdmin.settings();
			settings = cloneTranscodeSettings(response.settings);
			persistedSettings = cloneTranscodeSettings(response.settings);
			capabilities = response.capabilities;
			sessions = (await api.transcodeAdmin.sessions()).sessions;
		} catch (err) {
			error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
		refreshTimer = setInterval(() => void refreshSessions(), 5000);
		return () => {
			if (refreshTimer) clearInterval(refreshTimer);
		};
	});

	onDestroy(() => {
		if (refreshTimer) clearInterval(refreshTimer);
	});

	async function refreshSessions(): Promise<void> {
		try {
			sessions = (await api.transcodeAdmin.sessions()).sessions;
		} catch {
			// A transient failure just keeps the last list.
		}
	}

	async function save(): Promise<void> {
		const current = settings;
		if (!current) return;
		saving = true;
		try {
			const response = await api.transcodeAdmin.saveSettings(current);
			settings = cloneTranscodeSettings(response.settings);
			persistedSettings = cloneTranscodeSettings(response.settings);
			capabilities = response.capabilities;
			toasts.success('Transcoding settings saved. New sessions use them immediately.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not save the settings.'));
		} finally {
			saving = false;
		}
	}

	async function cancelSession(session: string): Promise<void> {
		sessionBusy = session;
		try {
			await api.transcodeAdmin.cancelSession(session);
			sessions = sessions.filter((s) => s.session !== session);
			toasts.success('Session stopped.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not stop the session.'));
		} finally {
			sessionBusy = null;
		}
	}

	function addQuality(): void {
		if (!settings) return;
		settings.qualities = [
			...settings.qualities,
			{ name: 'custom', max_width: 1920, max_height: 1080, bitrate_kbps: 6000 }
		];
	}

	function removeQuality(index: number): void {
		if (!settings) return;
		settings.qualities = settings.qualities.filter((_, i) => i !== index);
	}

	function numberValue(event: Event): number {
		const value = Number((event.currentTarget as HTMLInputElement).value);
		return Number.isFinite(value) ? value : 0;
	}

	function resetChanges(): void {
		if (!persistedSettings) return;
		settings = cloneTranscodeSettings(persistedSettings);
		toasts.info('Unsaved playback changes were reset.');
	}

	function stageAutomaticPolicy(): void {
		if (!settings) return;
		settings = applySafeAutomaticPolicy(settings, capabilities);
		toasts.info('Automatic settings are staged from the server probe. Review and save when ready.');
	}

	function stageHardware(backend: string): void {
		if (!settings || !readyHardware.includes(backend)) return;
		settings.hardware_acceleration = backend;
		settings.hardware_encode = true;
		toasts.info(`${backend.toUpperCase()} is staged. The server will still fall back visibly if it fails.`);
	}

	function stageSoftware(): void {
		if (!settings) return;
		settings.hardware_acceleration = 'none';
		settings.hardware_encode = false;
		settings.hardware_low_power = false;
	}

	function sectionMatches(...keywords: string[]): boolean {
		if (!normalizedSearch) return true;
		return keywords.some((keyword) => keyword.toLowerCase().includes(normalizedSearch));
	}

	function scrollToSection(id: string): void {
		document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

	function setQualityField(index: number, field: keyof TranscodeQuality, value: string | number): void {
		if (!settings) return;
		const next = [...settings.qualities];
		next[index] = { ...next[index], [field]: value };
		settings.qualities = next;
	}

	function toggleCodec(codec: string, enabled: boolean): void {
		if (!settings) return;
		const set = new Set(settings.hardware_decode_codecs);
		if (enabled) set.add(codec);
		else set.delete(codec);
		settings.hardware_decode_codecs = decodeCodecs.filter((c) => set.has(c));
	}

	function formatBytes(bytes: number): string {
		const gib = bytes / 1024 ** 3;
		return `${gib.toFixed(gib >= 10 ? 0 : 1)} GiB`;
	}
</script>

<svelte:head><title>Playback — Settings — Lain</title></svelte:head>

<div class="mx-auto max-w-[1500px] space-y-5">
	{#if loading}
		<div class="grid gap-4 lg:grid-cols-[16rem_1fr]">
			<div class="h-72 animate-pulse rounded-card bg-surface-active/50"></div>
			<div class="h-96 animate-pulse rounded-card bg-surface-active/50"></div>
		</div>
	{:else if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if settings}
		<header class="flex flex-col gap-3 border-b border-line/70 pb-5 lg:flex-row lg:items-end lg:justify-between">
			<div>
				<p class="font-mono text-[11px] uppercase tracking-[0.18em] text-accent">Playback policy</p>
				<h2 class="mt-2 text-2xl font-semibold tracking-tight text-foreground">A clear path from file to screen.</h2>
				<p class="mt-1 max-w-3xl text-sm leading-6 text-muted">
					Set the everyday behavior here. Expert controls remain available below, without competing
					with the choices that affect every stream.
				</p>
			</div>
			<div class="flex items-center gap-2 lg:justify-end">
				<span class="size-2 rounded-full bg-success"></span>
				<p class="max-w-sm text-xs leading-5 text-muted lg:text-right">
					Saved policy · changes affect new sessions; running sessions keep their current policy.
				</p>
			</div>
		</header>

		<div class="grid items-start gap-5 xl:grid-cols-[16rem_minmax(0,1fr)]">
			<aside class="min-w-0 space-y-4 xl:sticky xl:top-24">
				<section class="overflow-hidden rounded-card border border-accent/30 bg-accent/5">
					<div class="border-b border-accent/20 p-4">
						<div class="flex items-center gap-2 text-accent">
							<Sparkles class="size-4" />
							<h2 class="text-sm font-semibold">Automatic setup</h2>
						</div>
						<p class="mt-2 text-xs leading-5 text-muted">
							Stages HLS, browser-safe codecs, detected hardware, HDR, subtitles, deinterlacing and thread count.
							The best backend that passed the server probe is selected automatically.
						</p>
					</div>
					<div class="p-3">
						<Button class="w-full" variant="secondary" size="sm" onclick={stageAutomaticPolicy}>
							<WandSparkles class="size-4" /> Apply Auto
						</Button>
					</div>
				</section>

				{#if readyHardware.length > 0}
					<section class="rounded-card border border-line bg-surface/40 p-4">
						<p class="font-mono text-[10px] uppercase tracking-[0.16em] text-muted">Acceleration found</p>
						<p class="mt-2 text-sm font-medium text-foreground">
							{readyHardware.map((backend) => backend.toUpperCase()).join(' · ')}
						</p>
						<p class="mt-1 text-xs leading-5 text-muted">The first ready backend is automatic; you can override it here.</p>
						<div class="mt-3 flex flex-wrap gap-2">
							{#each readyHardware as backend (backend)}
								<Button variant="ghost" size="sm" onclick={() => stageHardware(backend)}>
									Use {backend.toUpperCase()}
								</Button>
							{/each}
						</div>
					</section>
				{/if}

				<div class="relative">
					<Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted" />
					<input
						bind:value={searchQuery}
						aria-label="Find a playback setting"
						placeholder="Find a setting…"
						class="h-10 w-full rounded-md border border-line bg-surface pl-9 pr-3 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none"
					/>
				</div>

				<nav class="no-scrollbar flex gap-1 overflow-x-auto pb-1 xl:block xl:space-y-1" aria-label="Playback settings sections">
					{#each [
						['overview', 'Overview'],
						['delivery', 'Delivery'],
						['encoding', 'Encoding'],
						['hardware', 'Hardware'],
						['processing', 'HDR & processing'],
						['audio', 'Audio & subtitles'],
						['resources', 'Resources'],
						['sessions', 'Active sessions']
					] as item (item[0])}
						<button
							type="button"
							onclick={() => scrollToSection(item[0])}
							class="flex shrink-0 items-center gap-2 rounded-md px-3 py-2 text-left text-xs font-medium text-muted transition-colors hover:bg-surface-hover hover:text-foreground xl:w-full"
						>
							<span class="size-1 rounded-full bg-line"></span>{item[1]}
						</button>
					{/each}
				</nav>
			</aside>

			<main class="min-w-0 space-y-5">
				<section id="overview" class="scroll-mt-24 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
					<div class="rounded-card border border-line bg-surface/40 p-4">
						<Radio class="size-4 text-accent" />
						<p class="mt-4 text-xs text-muted">Default stream</p>
						<p class="mt-1 text-sm font-semibold text-foreground">{settings.default_delivery === 'hls' ? 'HLS · starts early' : 'Progressive · full file'}</p>
					</div>
					<div class="rounded-card border border-line bg-surface/40 p-4">
						<Cpu class="size-4 text-accent" />
						<p class="mt-4 text-xs text-muted">Video engine</p>
						<p class="mt-1 text-sm font-semibold text-foreground">{hardwareActive ? settings.hardware_acceleration.toUpperCase() : 'Software-safe'}</p>
					</div>
					<div class="rounded-card border border-line bg-surface/40 p-4">
						<Gauge class="size-4 text-accent" />
						<p class="mt-4 text-xs text-muted">HDR policy</p>
						<p class="mt-1 text-sm font-semibold text-foreground">{settings.tone_mapping ? (settings.tone_mapping_mode === 'auto' ? 'Automatic' : settings.tone_mapping_mode) : 'Disabled'}</p>
					</div>
					<div class="rounded-card border border-line bg-surface/40 p-4">
						<Activity class="size-4 text-accent" />
						<p class="mt-4 text-xs text-muted">Now preparing</p>
						<p class="mt-1 text-sm font-semibold text-foreground">{activeSessions} active {activeSessions === 1 ? 'session' : 'sessions'}</p>
					</div>
				</section>

				{#if capabilities && sectionMatches('diagnostics', 'capabilities', 'ffmpeg', 'encoder', 'probe')}
					<details class="group rounded-card border border-line bg-surface/30" open={!!normalizedSearch}>
						<summary class="flex cursor-pointer list-none items-center justify-between gap-3 px-4 py-3">
							<div class="flex items-center gap-2">
								<Cpu class="size-4 text-muted" />
								<div>
									<h2 class="text-sm font-semibold text-foreground">Probed capabilities</h2>
									<p class="text-xs text-muted">Detailed diagnostics from the server's current ffmpeg.</p>
								</div>
							</div>
							<Badge tone={capabilities.tone_mapping ? 'success' : 'warning'}>{capabilities.tone_mapping ? 'ready' : 'limited'}</Badge>
						</summary>
						<div class="border-t border-line px-4 py-4">
							<p class="text-xs text-muted">ffmpeg: {capabilities.ffmpeg || 'system PATH'}</p>
							<div class="mt-3 flex flex-wrap gap-1.5">
								<Badge tone={capabilities.tone_mapping ? 'success' : 'danger'}>tone mapping {capabilities.tone_mapping ? 'available' : 'unavailable'}</Badge>
								<Badge tone={capabilities.tone_mapping_bt2390 ? 'success' : 'neutral'}>BT.2390 {capabilities.tone_mapping_bt2390 ? 'available' : 'fallback to hable'}</Badge>
								{#each Object.entries(capabilities.hardware) as [backend, ok] (backend)}
									<Badge tone={ok ? 'success' : 'neutral'}>{backend} {ok ? 'ready' : 'not available'}</Badge>
								{/each}
							</div>
							<p class="mt-4 font-mono text-[10px] uppercase tracking-[0.16em] text-muted">Encoders</p>
							<div class="mt-2 flex flex-wrap gap-1.5">
								{#each capabilities.encoders as encoder (encoder)}<Badge>{encoder}</Badge>{/each}
							</div>
						</div>
					</details>
				{/if}

		<!-- Delivery -->
		{#if sectionMatches('delivery', 'stream', 'hls', 'segment', 'timeout', 'throttle')}
		<section id="delivery" class="scroll-mt-24 space-y-5 rounded-card border border-line bg-surface/40 p-5">
			<div class="flex items-center gap-2">
				<Clapperboard class="size-4 text-muted" />
				<div>
					<h2 class="text-sm font-semibold text-foreground">Delivery</h2>
					<p class="mt-0.5 text-xs text-muted">How playback starts, advances and releases work.</p>
				</div>
			</div>
			<Select
				label="Default delivery"
				value={settings.default_delivery}
				options={deliveries}
				onValueChange={(value) => (settings!.default_delivery = value as TranscodeSettings['default_delivery'])}
			/>
			<Select
				label="HLS segment container"
				value={settings.hls_segment_container}
				options={containers}
				onValueChange={(value) => (settings!.hls_segment_container = value)}
			/>
			<div class="grid gap-4 sm:grid-cols-2">
				<Input
					label="HLS segment length (seconds)"
					type="number"
					min="1"
					max="30"
					value={String(settings.hls_segment_seconds)}
					oninput={(e) => (settings!.hls_segment_seconds = numberValue(e))}
				/>
				<Input
					label="Idle timeout (seconds)"
					type="number"
					min="15"
					max="3600"
					value={String(settings.idle_timeout_sec)}
					oninput={(e) => (settings!.idle_timeout_sec = numberValue(e))}
					hint="Stop sessions nobody is watching anymore."
				/>
			</div>
			<Switch
				label="Throttle transcodes"
				description="Pause ffmpeg while the client is far behind the produced segments."
				checked={settings.throttle}
				onCheckedChange={(checked) => (settings!.throttle = checked)}
			/>
			{#if settings.throttle}
				<Input
					label="Throttle delay — produced-ahead budget (seconds)"
					type="number"
					min="5"
					max="600"
					value={String(settings.throttle_ahead_sec)}
					oninput={(e) => (settings!.throttle_ahead_sec = numberValue(e))}
					hint="Jellyfin calls this the throttle delay: how far ahead of the viewer ffmpeg may run."
				/>
			{/if}
			<Switch
				label="Enable segment deletion"
				description="Delete HLS segments once the client is done with them; rewinding beyond the kept window needs a fresh session."
				checked={settings.segment_deletion}
				onCheckedChange={(checked) => (settings!.segment_deletion = checked)}
			/>
			{#if settings.segment_deletion}
				<Input
					label="Segment keep window (seconds)"
					type="number"
					min="0"
					max="3600"
					value={String(settings.segment_keep_sec)}
					oninput={(e) => (settings!.segment_keep_sec = numberValue(e))}
				/>
			{/if}
		</section>
		{/if}

		<!-- Encoding -->
		{#if sectionMatches('encoding', 'quality', 'codec', 'crf', 'preset', 'hevc', 'av1', 'deinterlace')}
		<section id="encoding" class="scroll-mt-24 space-y-5 rounded-card border border-line bg-surface/40 p-5">
			<div class="flex items-center gap-2">
				<Gauge class="size-4 text-muted" />
				<div>
					<h2 class="text-sm font-semibold text-foreground">Encoding</h2>
					<p class="mt-0.5 text-xs text-muted">Quality defaults and the resolution ladder exposed to viewers.</p>
				</div>
			</div>
			<div class="grid gap-4 sm:grid-cols-2">
				<Select
					label="Encoder preset"
					value={settings.encoder_preset}
					options={presets.map((p) => ({ value: p, label: p }))}
					onValueChange={(value) => (settings!.encoder_preset = value)}
				/>
				<Input
					label="Quality (CRF, lower is better)"
					type="number"
					min="0"
					max="51"
					value={String(settings.crf)}
					oninput={(e) => (settings!.crf = numberValue(e))}
				/>
			</div>
			<Switch
				label="Allow HEVC (H.265) output"
				description="Per-request codec selection can then ask for HEVC; off by default for browser reach."
				checked={settings.allow_hevc}
				onCheckedChange={(checked) => (settings!.allow_hevc = checked)}
			/>
			<Switch
				label="Allow AV1 output"
				description="Requires a local AV1 encoder (libsvtav1/libaom-av1)."
				checked={settings.allow_av1}
				onCheckedChange={(checked) => (settings!.allow_av1 = checked)}
			/>
			<Switch
				label="Deinterlace when detected"
				checked={settings.deinterlace === 'auto'}
				onCheckedChange={(checked) => (settings!.deinterlace = checked ? 'auto' : 'off')}
			/>
			{#if settings.deinterlace === 'auto'}
				<Select
					label="Deinterlace method"
					value={settings.deinterlace_method}
					options={deinterlaceMethods}
					onValueChange={(value) => (settings!.deinterlace_method = value)}
				/>
				<Switch
					label="Deinterlace double rate"
					description="Keep both fields as frames: smoother motion, twice the output frame count."
					checked={settings.deinterlace_double_rate}
					onCheckedChange={(checked) => (settings!.deinterlace_double_rate = checked)}
				/>
			{/if}

			<div>
				<h3 class="text-sm font-medium text-muted">Per-codec encoding</h3>
				<p class="mt-1 text-xs text-muted">
					Each output codec has its own preset and CRF, like Jellyfin. Leave a preset on “inherit” to use
					the preset above.
				</p>
				<div class="mt-3 space-y-3">
					{#each codecRows as row (row.key)}
						<div class="grid items-end gap-2 sm:grid-cols-[6rem_1fr_8rem]">
							<p class="pb-2 text-sm font-medium text-foreground">{row.label}</p>
							<Select
								aria-label={`${row.label} preset`}
								value={row.preset()}
								options={[{ value: '', label: 'Inherit' }, ...presets.map((p) => ({ value: p, label: p }))]}
								onValueChange={(value) => row.setPreset(value)}
							/>
							<Input
								aria-label={`${row.label} CRF`}
								type="number"
								min="0"
								max="51"
								value={String(row.crf())}
								oninput={(e) => row.setCRF(numberValue(e))}
							/>
						</div>
					{/each}
				</div>
			</div>

			<div>
				<div class="flex items-center justify-between">
					<h3 class="text-sm font-medium text-muted">Quality ladder</h3>
					<Button variant="secondary" size="sm" onclick={addQuality}>Add entry</Button>
				</div>
				<div class="mt-2 space-y-2">
					{#each settings.qualities as quality, index (index)}
						<div class="grid grid-cols-2 gap-2 rounded-md border border-line bg-surface/60 p-2 sm:grid-cols-[1.2fr_1fr_1fr_1fr_auto]">
							<Input
								aria-label="Quality name"
								value={quality.name}
								oninput={(e) => setQualityField(index, 'name', (e.currentTarget as HTMLInputElement).value)}
							/>
							<Input
								aria-label="Max width"
								type="number"
								value={String(quality.max_width ?? 0)}
								oninput={(e) => setQualityField(index, 'max_width', numberValue(e))}
							/>
							<Input
								aria-label="Max height"
								type="number"
								value={String(quality.max_height ?? 0)}
								oninput={(e) => setQualityField(index, 'max_height', numberValue(e))}
							/>
							<Input
								aria-label="Bitrate kbps"
								type="number"
								value={String(quality.bitrate_kbps ?? 0)}
								oninput={(e) => setQualityField(index, 'bitrate_kbps', numberValue(e))}
							/>
							<Button variant="ghost" size="sm" onclick={() => removeQuality(index)} aria-label="Remove quality">
								<Trash2 class="size-4" />
							</Button>
						</div>
					{/each}
				</div>
				<p class="mt-1 text-xs text-muted">Name · max width · max height · bitrate (kbps).</p>
			</div>
		</section>
		{/if}

		<!-- Hardware -->
		{#if sectionMatches('hardware', 'acceleration', 'gpu', 'vaapi', 'nvenc', 'qsv', 'decode')}
		<details id="hardware" class="group scroll-mt-24 rounded-card border border-line bg-surface/40" open={hardwareActive || !!normalizedSearch}>
			<summary class="flex cursor-pointer list-none items-center justify-between gap-3 p-5">
				<div class="flex items-center gap-2">
					<Cpu class="size-4 text-muted" />
					<div>
						<h2 class="text-sm font-semibold text-foreground">Hardware acceleration</h2>
						<p class="mt-0.5 text-xs text-muted">Automatic GPU encode and decode, guarded by the server probe.</p>
					</div>
				</div>
				<Badge tone={hardwareActive ? 'accent' : 'neutral'}>{hardwareActive ? settings.hardware_acceleration.toUpperCase() : 'software'}</Badge>
			</summary>
			<div class="space-y-4 border-t border-line p-5">
			<Select
				label="Backend"
				value={settings.hardware_acceleration}
				options={probedHardwareOptions}
				onValueChange={(value) => (settings!.hardware_acceleration = value)}
			/>
			{#if settings.hardware_acceleration !== 'none'}
				<div class="flex flex-wrap items-center justify-between gap-3 rounded-md border border-accent/20 bg-accent/5 p-3">
					<p class="text-xs leading-5 text-muted">A failed runtime attempt falls back to software and is reported on the session.</p>
					<Button variant="ghost" size="sm" onclick={stageSoftware}>Use software</Button>
				</div>
			{/if}
			{#if settings.hardware_acceleration === 'vaapi'}
				<div class="space-y-1.5">
					<Input
						aria-label="VA-API device"
						value={settings.hardware_device}
						oninput={(e) => (settings!.hardware_device = (e.currentTarget as HTMLInputElement).value)}
					/>
					<p class="text-xs text-muted">
						VA-API render node (default /dev/dri/renderD128). Other backends pick their own device.
					</p>
				</div>
			{/if}
			<Switch
				label="Use hardware encoding"
				description="Encode with the selected backend when its probe passes; a failed attempt falls back to software visibly."
				checked={settings.hardware_encode}
				onCheckedChange={(checked) => (settings!.hardware_encode = checked)}
			/>
			{#if settings.hardware_acceleration === 'qsv'}
				<Switch
					label="Intel low-power encoder"
					description="Use QSV's low-power H.264/HEVC encoder (experimental; many mfx version and rate-control limitations)."
					checked={settings.hardware_low_power}
					onCheckedChange={(checked) => (settings!.hardware_low_power = checked)}
				/>
			{/if}
			<div>
				<p class="text-sm font-medium text-muted">Hardware decoding</p>
				<div class="mt-2 flex flex-wrap gap-3">
					{#each decodeCodecs as codec (codec)}
						<label class="flex items-center gap-1.5 text-sm text-foreground">
							<input
								type="checkbox"
								checked={settings.hardware_decode_codecs.includes(codec)}
								onchange={(e) => toggleCodec(codec, (e.currentTarget as HTMLInputElement).checked)}
							/>
							{codec}
						</label>
					{/each}
				</div>
			</div>
			<Switch
				label="Decode 10-bit HEVC in hardware"
				description="Off by default: some backends fail or produce wrong colours on 10-bit HEVC."
				checked={settings.hardware_decode_10bit_hevc}
				onCheckedChange={(checked) => (settings!.hardware_decode_10bit_hevc = checked)}
			/>
			<Switch
				label="Decode 10-bit VP9 in hardware"
				checked={settings.hardware_decode_10bit_vp9}
				onCheckedChange={(checked) => (settings!.hardware_decode_10bit_vp9 = checked)}
			/>
			</div>
		</details>
		{/if}

		<!-- HDR -->
		{#if sectionMatches('hdr', 'processing', 'tone mapping', 'luminance', 'deinterlace')}
		<section id="processing" class="scroll-mt-24 space-y-4 rounded-card border border-line bg-surface/40 p-5">
			<div class="flex items-center gap-2">
				<WandSparkles class="size-4 text-muted" />
				<div>
					<h2 class="text-sm font-semibold text-foreground">HDR & tone mapping</h2>
					<p class="mt-0.5 text-xs text-muted">Automatic is recommended: process HDR only when the source needs it.</p>
				</div>
			</div>
			<Switch
				label="Enable tone mapping"
				description="HDR sources become SDR when the chain executes; without it HDR browser playback stays unavailable."
				checked={settings.tone_mapping}
				onCheckedChange={(checked) => (settings!.tone_mapping = checked)}
			/>
			{#if settings.tone_mapping}
				<div class="grid gap-4 sm:grid-cols-2">
					<Select
						label="Algorithm"
						value={settings.tone_mapping_algorithm}
						options={toneAlgorithms}
						onValueChange={(value) => (settings!.tone_mapping_algorithm = value)}
					/>
					<Select
						label="When to apply"
						value={settings.tone_mapping_mode}
						options={toneModes}
						onValueChange={(value) => (settings!.tone_mapping_mode = value)}
					/>
				</div>
				<Input
					label="Nominal peak luminance (nits)"
					type="number"
					min="50"
					max="10000"
					value={String(settings.tone_mapping_peak_nits)}
					oninput={(e) => (settings!.tone_mapping_peak_nits = numberValue(e))}
				/>
			{/if}
		</section>
		{/if}

		<!-- Audio & subtitles -->
		{#if sectionMatches('audio', 'subtitle', 'captions', 'downmix', 'font', 'bitrate')}
		<section id="audio" class="scroll-mt-24 space-y-4 rounded-card border border-line bg-surface/40 p-5">
			<div class="flex items-center gap-2">
				<AudioLines class="size-4 text-muted" />
				<div>
					<h2 class="text-sm font-semibold text-foreground">Audio & subtitles</h2>
					<p class="mt-0.5 text-xs text-muted">Prefer extraction and direct playback; burn only when a track requires it.</p>
				</div>
			</div>
			<div class="grid gap-4 sm:grid-cols-2">
				<Input
					label="Audio bitrate (kbps)"
					type="number"
					min="32"
					max="640"
					value={String(settings.audio_bitrate_kbps)}
					oninput={(e) => (settings!.audio_bitrate_kbps = numberValue(e))}
				/>
				<Select
					label="Subtitle handling"
					value={settings.subtitle_mode}
					options={subtitleModes}
					onValueChange={(value) => (settings!.subtitle_mode = value as TranscodeSettings['subtitle_mode'])}
				/>
			</div>
			<Switch
				label="Variable-bitrate audio (AAC VBR)"
			description="Uses ffmpeg's native VBR quality step mapped from the bitrate above instead of a fixed rate."
				checked={settings.audio_vbr}
				onCheckedChange={(checked) => (settings!.audio_vbr = checked)}
			/>
			<Switch
				label="Allow subtitle extraction on the fly"
				description="Serves WebVTT for a chosen track while a file plays directly, without transcoding it."
				checked={settings.allow_subtitle_extraction !== false}
				onCheckedChange={(checked) => (settings!.allow_subtitle_extraction = checked)}
			/>
			<Switch
				label="Downmix multichannel audio"
				description="More than two channels become stereo AAC."
				checked={settings.downmix_audio}
				onCheckedChange={(checked) => (settings!.downmix_audio = checked)}
			/>
			{#if settings.downmix_audio}
				<div class="grid gap-4 sm:grid-cols-2">
					<Select
						label="Stereo downmix algorithm"
						value={settings.downmix_stereo_algorithm}
						options={downmixAlgorithms}
						onValueChange={(value) => (settings!.downmix_stereo_algorithm = value)}
					/>
					<Input
						label="Downmix gain (linear)"
						type="number"
						min="0.5"
						max="8"
						step="0.5"
						value={String(settings.downmix_audio_boost)}
						oninput={(e) => (settings!.downmix_audio_boost = numberValue(e))}
						hint="Applied only when downmixing; 1 leaves the level untouched."
					/>
				</div>
			{/if}
			<Switch
				label="Enable fallback fonts"
				description="Use the font directory and forced family below when a burned-in subtitle needs a font."
				checked={settings.fallback_font_enabled !== false}
				onCheckedChange={(checked) => (settings!.fallback_font_enabled = checked)}
			/>
			<div class="grid gap-4 sm:grid-cols-2">
				<Input
					label="Burn-in font directory"
					placeholder="system font path"
					value={settings.fallback_font_path}
					oninput={(e) => (settings!.fallback_font_path = (e.currentTarget as HTMLInputElement).value)}
					hint="Searched first when a burned-in subtitle needs a font."
				/>
				<Input
					label="Forced font family"
					placeholder="e.g. DejaVu Sans"
					value={settings.fallback_font_name}
					oninput={(e) => (settings!.fallback_font_name = (e.currentTarget as HTMLInputElement).value)}
				/>
			</div>
		</section>
		{/if}

		<!-- Resources & paths -->
		{#if sectionMatches('performance', 'resources', 'storage', 'cache', 'queue', 'path', 'ffmpeg', 'threads', 'bitrate')}
		<details id="resources" class="group scroll-mt-24 rounded-card border border-line bg-surface/40" open={customResourcesActive || !!normalizedSearch}>
			<summary class="flex cursor-pointer list-none items-center justify-between gap-3 p-5">
				<div class="flex items-center gap-2">
					<HardDrive class="size-4 text-muted" />
					<div>
						<h2 class="text-sm font-semibold text-foreground">Performance, resources & storage</h2>
						<p class="mt-0.5 text-xs text-muted">Capacity limits, cache, process paths and temporary storage.</p>
					</div>
				</div>
				<Badge tone={customResourcesActive ? 'accent' : 'neutral'}>{customResourcesActive ? 'configured' : 'automatic'}</Badge>
			</summary>
			<div class="space-y-4 border-t border-line p-5">
			<div class="grid gap-4 sm:grid-cols-2">
				<Input
					label="Transcoding thread count (0 = auto)"
					type="number"
					min="0"
					max="64"
					value={String(settings.thread_count)}
					oninput={(e) => (settings!.thread_count = numberValue(e))}
				/>
				<Input
					label="Max muxing queue size (packets)"
					type="number"
					min="128"
					max="65536"
					value={String(settings.max_muxing_queue_size)}
					oninput={(e) => (settings!.max_muxing_queue_size = numberValue(e))}
					hint="Buffers output packets so a slow client cannot abort the muxer."
				/>
			</div>
			<Input
				label="Transcoding temporary path"
				placeholder="data dir (default)"
				value={settings.transcode_temp_path}
				oninput={(e) => (settings!.transcode_temp_path = (e.currentTarget as HTMLInputElement).value)}
				hint="Artifacts move to <path>/lain-transcode/<session>/; the JSON index stays in the data dir."
			/>
			<Input
				label="Remote client bitrate limit (kbps, 0 = unlimited)"
				type="number"
				min="0"
				max="1000000"
				value={String(settings.remote_bitrate_limit_kbps)}
				oninput={(e) => (settings!.remote_bitrate_limit_kbps = numberValue(e))}
				hint="Server-wide ceiling; the tighter of this and a user's own limit wins."
			/>
			<div class="grid gap-4 sm:grid-cols-2">
				<Input
					label={`Cache budget (bytes — ${formatBytes(settings.cache_bytes)})`}
					type="number"
					value={String(settings.cache_bytes)}
					oninput={(e) => (settings!.cache_bytes = numberValue(e))}
				/>
				<Input
					label="Queue size"
					type="number"
					min="1"
					max="64"
					value={String(settings.queue_size)}
					oninput={(e) => (settings!.queue_size = numberValue(e))}
				/>
				<Input
					label="Max concurrent sessions"
					type="number"
					min="1"
					max="16"
					value={String(settings.max_concurrent)}
					oninput={(e) => (settings!.max_concurrent = numberValue(e))}
				/>
			</div>
			<div class="grid gap-4 sm:grid-cols-2">
				<Input
					label="ffmpeg path"
					placeholder="system PATH"
					value={settings.ffmpeg_path}
					oninput={(e) => (settings!.ffmpeg_path = (e.currentTarget as HTMLInputElement).value)}
				/>
				<Input
					label="ffprobe path"
					placeholder="system PATH"
					value={settings.ffprobe_path}
					oninput={(e) => (settings!.ffprobe_path = (e.currentTarget as HTMLInputElement).value)}
				/>
			</div>
			</div>
		</details>
		{/if}

		<!-- Sessions -->
		{#if sectionMatches('active sessions', 'session', 'running', 'queue', 'fps', 'encoder')}
		<section id="sessions" class="scroll-mt-24 space-y-3 rounded-card border border-line bg-surface/40 p-5">
			<div class="flex items-center justify-between">
				<h2 class="text-sm font-semibold text-foreground">Active sessions</h2>
				<Button variant="ghost" size="sm" onclick={() => void refreshSessions()}>Refresh</Button>
			</div>
			{#if sessions.length === 0}
				<p class="text-sm text-muted">Nothing is being prepared right now.</p>
			{:else}
				<ul class="divide-y divide-line overflow-hidden rounded-md border border-line">
					{#each sessions as session (session.session)}
						<li class="flex flex-wrap items-center gap-3 bg-surface/60 px-3 py-2 text-sm">
							<Badge tone={session.state === 'failed' ? 'danger' : session.state === 'ready' ? 'success' : 'neutral'}>
								{session.state}
							</Badge>
							<span class="font-mono text-xs text-muted">{session.session.slice(0, 12)}</span>
							<span class="text-xs text-muted">
								{session.delivery ?? 'progressive'} · {session.method ?? '—'}
								{#if session.encoder}· {session.encoder}{/if}
								{#if session.hardware}· {session.hardware}{/if}
								{#if session.progress}· {Math.round(session.progress * 100)}%{/if}
								{#if session.fps}· {session.fps.toFixed(1)} fps{/if}
								{#if session.output_bitrate_kbps}· {session.output_bitrate_kbps} kbps{/if}
							</span>
							{#if session.fallback}
								<span class="text-xs text-warning">{session.fallback}</span>
							{/if}
							<div class="ml-auto">
								{#if sessionBusy === session.session}
									<Spinner class="size-4 text-muted" />
								{:else}
									<Button variant="ghost" size="sm" onclick={() => void cancelSession(session.session)}>
										Stop
									</Button>
								{/if}
							</div>
						</li>
					{/each}
				</ul>
			{/if}
		</section>
		{/if}

				{#if normalizedSearch && !hasSearchResults}
					<div class="rounded-card border border-dashed border-line p-8 text-center">
						<Search class="mx-auto size-5 text-muted" />
						<p class="mt-3 text-sm font-medium text-foreground">No setting matches “{searchQuery.trim()}”</p>
						<p class="mt-1 text-xs text-muted">Try a feature, codec, resource or delivery term.</p>
					</div>
				{/if}
			</main>
		</div>

		{#if dirty}
			<div class="fixed bottom-20 left-4 right-4 z-40 flex items-center justify-between gap-3 rounded-card border border-warning/40 bg-background/95 p-3 shadow-2xl backdrop-blur sm:bottom-6 sm:left-auto sm:right-6 sm:w-[min(30rem,calc(100vw-3rem))]">
				<div class="flex min-w-0 items-center gap-3">
					<span class="size-2 shrink-0 rounded-full bg-warning"></span>
					<div class="min-w-0">
						<p class="text-sm font-medium text-foreground">Unsaved changes</p>
						<p class="truncate text-xs text-muted">Review, reset or save from anywhere on this page.</p>
					</div>
				</div>
				<div class="flex shrink-0 items-center gap-1">
					<Button variant="ghost" size="sm" disabled={saving} onclick={resetChanges}>
						<RotateCcw class="size-4" /> Reset
					</Button>
					<Button size="sm" loading={saving} onclick={() => void save()}>
						<Save class="size-4" /> Save
					</Button>
				</div>
			</div>
		{/if}
	{/if}
</div>
