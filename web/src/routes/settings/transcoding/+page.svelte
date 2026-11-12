<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import Plus from '@lucide/svelte/icons/plus';
	import Search from '@lucide/svelte/icons/search';
	import Trash2 from '@lucide/svelte/icons/trash-2';
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
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import {
		applySafeAutomaticPolicy,
		availableHardwareBackends,
		cloneTranscodeSettings,
		engineChoices,
		settingsEqual
	} from '$lib/settings/playback-policy';
	import { page } from '$app/state';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import StagedBar from '$lib/components/settings/StagedBar.svelte';
	import { t } from '$lib/i18n';
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
	const engines = $derived(engineChoices(settings?.hardware_acceleration ?? 'none', readyHardware));
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
			// The mode shown is the truth about the saved policy: Auto only
			// when it equals what Auto would stage from this probe.
			const auto = applySafeAutomaticPolicy(cloneTranscodeSettings(response.settings), response.capabilities);
			if (!CUSTOM_ANCHORS.includes(page.url.hash.slice(1)))
				view = settingsEqual(auto, response.settings) ? 'auto' : 'custom';
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

	// Auto shows a summary; Custom reveals every section. A search or a
	// Ctrl+K jump to a section always reveals it.
	const CUSTOM_ANCHORS = ['delivery', 'encoding', 'hardware', 'processing', 'audio', 'resources'];
	let view = $state<'auto' | 'custom'>('auto');
	$effect(() => {
		if (CUSTOM_ANCHORS.includes(page.url.hash.slice(1))) view = 'custom';
	});
	function setView(next: 'auto' | 'custom'): void {
		view = next;
		if (next === 'auto') stageAutomaticPolicy();
	}

	const changeCount = $derived.by(() => {
		if (!settings || !persistedSettings) return 0;
		const a = settings as unknown as Record<string, unknown>;
		const b = persistedSettings as unknown as Record<string, unknown>;
		return Object.keys({ ...a, ...b }).filter((k) => JSON.stringify(a[k]) !== JSON.stringify(b[k])).length;
	});

	function searchMatches(...keywords: string[]): boolean {
		if (!normalizedSearch) return true;
		return keywords.some((keyword) => keyword.toLowerCase().includes(normalizedSearch));
	}

	function sectionMatches(...keywords: string[]): boolean {
		if (!normalizedSearch) return view === 'custom';
		return keywords.some((keyword) => keyword.toLowerCase().includes(normalizedSearch));
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

	// "/" jumps to the setting search, as in most developer tools.
	let searchInput = $state<HTMLInputElement>();
	function onSlash(e: KeyboardEvent): void {
		const t = e.target as HTMLElement | null;
		if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return;
		if (t && (t.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName))) return;
		e.preventDefault();
		searchInput?.focus();
	}
</script>

<svelte:window onkeydown={onSlash} />

<svelte:head><title>Transcoding — Settings — Lain</title></svelte:head>

<!-- Every field on this page is a row (label and hint left, control right)
     in the group it belongs to; numbers are one width and read right-aligned. -->
{#snippet numberInput(label: string, value: number, set: (n: number) => void, min?: number, max?: number, step?: number, wide = false)}
	<input
		type="number"
		class="h-9 {wide ? 'w-40' : 'w-28'} rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none"
		aria-label={label}
		{min}
		{max}
		{step}
		value={String(value)}
		oninput={(e) => set(numberValue(e))}
	/>
{/snippet}

{#snippet textInput(label: string, value: string, set: (v: string) => void, placeholder = '')}
	<input
		class="h-9 w-full rounded-md border px-3 text-sm text-foreground outline-none placeholder:text-muted/60"
		aria-label={label}
		{placeholder}
		{value}
		oninput={(e) => set((e.currentTarget as HTMLInputElement).value)}
	/>
{/snippet}

{#snippet select(label: string, value: string, options: { value: string; label: string; disabled?: boolean }[], set: (v: string) => void)}
	<div class="w-full">
		<Select aria-label={label} {value} {options} onValueChange={set} />
	</div>
{/snippet}

<div>
	{#if loading}
		<div class="space-y-3">
			{#each [0, 1, 2, 3] as i (i)}<div class="h-12 animate-pulse rounded-md bg-panel"></div>{/each}
		</div>
	{:else if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if settings}
		<div class="relative mb-10">
			<Search class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted" />
			<input
				bind:this={searchInput}
				bind:value={searchQuery}
				aria-label="Find a playback setting"
				placeholder="Filter transcoding settings"
				class="h-10 w-full rounded-md border pl-9 pr-10 text-sm text-foreground placeholder:text-muted/60 focus:outline-none"
			/>
			<kbd class="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2">/</kbd>
		</div>

		<SettingsGroup id="mode" title="Mode" description={t('settings.groups.transcoding.mode')}>
			<SettingRow
				label="Policy"
				hint={view === 'auto'
					? 'Lain picks delivery, codecs, hardware and HDR from the server probe.'
					: 'Every section is yours to tune. Changes apply to new sessions.'}
			>
				<div class="inline-flex rounded-md bg-field p-0.5" role="radiogroup" aria-label="Transcoding policy">
					{#each [['auto', 'Auto'], ['custom', 'Custom']] as [id, text] (id)}
						<button
							type="button"
							role="radio"
							aria-checked={view === id}
							class="h-8 rounded px-3 font-mono text-[11px] uppercase tracking-[0.14em] transition-colors {view === id
								? 'bg-foreground/[0.12] text-foreground'
								: 'text-muted hover:text-foreground'}"
							onclick={() => setView(id as 'auto' | 'custom')}
						>
							{text}
						</button>
					{/each}
				</div>
			</SettingRow>
			<SettingRow label="Video engine" hint="The first backend that passed the probe is used; failures fall back to software visibly.">
				<!-- One segmented choice, drawn as the policy's above: the engine in use
				     and the ones the probe passed. A word beside two bare "Use …" buttons
				     read as a label and two links, not as one control. -->
				<div class="inline-flex flex-wrap rounded-md bg-field p-0.5" role="radiogroup" aria-label="Video engine">
					{#each engines as backend (backend)}
						{@const on = (hardwareActive ? settings.hardware_acceleration : 'none') === backend}
						<button
							type="button"
							role="radio"
							aria-checked={on}
							class="h-8 rounded px-3 font-mono text-[11px] uppercase tracking-[0.14em] transition-colors {on
								? 'bg-foreground/[0.12] text-foreground'
								: 'text-muted hover:text-foreground'}"
							onclick={() => {
								if (on) return;
								if (backend === 'none') stageSoftware();
								else stageHardware(backend);
							}}
						>
							{backend === 'none' ? 'software' : backend}
						</button>
					{/each}
				</div>
			</SettingRow>
			<SettingRow label="Delivery">
				<span class="font-mono text-xs text-foreground">{settings.default_delivery === 'hls' ? 'HLS · starts early' : 'progressive · full file'}</span>
			</SettingRow>
			<SettingRow label="HDR">
				<span class="font-mono text-xs text-foreground">
					{settings.tone_mapping ? (settings.tone_mapping_mode === 'auto' ? 'tone map when needed' : settings.tone_mapping_mode) : 'off'}
				</span>
			</SettingRow>
			<SettingRow label="Now preparing">
				<a href="#sessions" class="font-mono text-xs text-foreground hover:text-accent">
					{activeSessions} {activeSessions === 1 ? 'session' : 'sessions'} ›
				</a>
			</SettingRow>
		</SettingsGroup>

		{#if capabilities && searchMatches('diagnostics', 'capabilities', 'ffmpeg', 'encoder', 'probe')}
			<SettingsGroup
				id="capabilities"
				title="Probed capabilities"
				description="Detailed diagnostics from the server's current ffmpeg."
				open={!!normalizedSearch}
			>
				{#snippet aside()}
					<Badge tone={capabilities?.tone_mapping ? 'success' : 'warning'}>{capabilities?.tone_mapping ? 'ready' : 'limited'}</Badge>
				{/snippet}
				<div class="py-4">
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
			</SettingsGroup>
		{/if}

		{#if sectionMatches('delivery', 'stream', 'hls', 'segment', 'timeout', 'throttle')}
			<SettingsGroup id="delivery" title="Delivery" description="How playback starts, advances and releases work.">
				<SettingRow label="Default delivery">
					{@render select('Default delivery', settings.default_delivery, deliveries, (value) => (settings!.default_delivery = value as TranscodeSettings['default_delivery']))}
				</SettingRow>
				<SettingRow label="HLS segment container">
					{@render select('HLS segment container', settings.hls_segment_container, containers, (value) => (settings!.hls_segment_container = value))}
				</SettingRow>
				<SettingRow label="HLS segment length (seconds)">
					{@render numberInput('HLS segment length (seconds)', settings.hls_segment_seconds, (n) => (settings!.hls_segment_seconds = n), 1, 30)}
				</SettingRow>
				<SettingRow label="Idle timeout (seconds)" hint="Stop sessions nobody is watching anymore.">
					{@render numberInput('Idle timeout (seconds)', settings.idle_timeout_sec, (n) => (settings!.idle_timeout_sec = n), 15, 3600)}
				</SettingRow>
				<SettingRow label="Throttle transcodes" hint="Pause ffmpeg while the client is far behind the produced segments.">
					<Switch bare label="Throttle transcodes" checked={settings.throttle} onCheckedChange={(checked) => (settings!.throttle = checked)} />
				</SettingRow>
				{#if settings.throttle}
					<SettingRow
						label="Throttle delay — produced-ahead budget (seconds)"
						hint="Jellyfin calls this the throttle delay: how far ahead of the viewer ffmpeg may run."
					>
						{@render numberInput('Throttle delay — produced-ahead budget (seconds)', settings.throttle_ahead_sec, (n) => (settings!.throttle_ahead_sec = n), 5, 600)}
					</SettingRow>
				{/if}
				<SettingRow
					label="Enable segment deletion"
					hint="Delete HLS segments once the client is done with them; rewinding beyond the kept window needs a fresh session."
				>
					<Switch bare label="Enable segment deletion" checked={settings.segment_deletion} onCheckedChange={(checked) => (settings!.segment_deletion = checked)} />
				</SettingRow>
				{#if settings.segment_deletion}
					<SettingRow label="Segment keep window (seconds)">
						{@render numberInput('Segment keep window (seconds)', settings.segment_keep_sec, (n) => (settings!.segment_keep_sec = n), 0, 3600)}
					</SettingRow>
				{/if}
			</SettingsGroup>
		{/if}

		{#if sectionMatches('encoding', 'quality', 'codec', 'crf', 'preset', 'hevc', 'av1', 'deinterlace')}
			<SettingsGroup id="encoding" title="Encoding" description="Quality defaults and the resolution ladder exposed to viewers.">
				<SettingRow label="Encoder preset">
					{@render select('Encoder preset', settings.encoder_preset, presets.map((p) => ({ value: p, label: p })), (value) => (settings!.encoder_preset = value))}
				</SettingRow>
				<SettingRow label="Quality (CRF, lower is better)">
					{@render numberInput('Quality (CRF, lower is better)', settings.crf, (n) => (settings!.crf = n), 0, 51)}
				</SettingRow>
				<SettingRow label="Allow HEVC (H.265) output" hint="Per-request codec selection can then ask for HEVC; off by default for browser reach.">
					<Switch bare label="Allow HEVC (H.265) output" checked={settings.allow_hevc} onCheckedChange={(checked) => (settings!.allow_hevc = checked)} />
				</SettingRow>
				<SettingRow label="Allow AV1 output" hint="Requires a local AV1 encoder (libsvtav1/libaom-av1).">
					<Switch bare label="Allow AV1 output" checked={settings.allow_av1} onCheckedChange={(checked) => (settings!.allow_av1 = checked)} />
				</SettingRow>
				<SettingRow label="Deinterlace when detected">
					<Switch bare label="Deinterlace when detected" checked={settings.deinterlace === 'auto'} onCheckedChange={(checked) => (settings!.deinterlace = checked ? 'auto' : 'off')} />
				</SettingRow>
				{#if settings.deinterlace === 'auto'}
					<SettingRow label="Deinterlace method">
						{@render select('Deinterlace method', settings.deinterlace_method, deinterlaceMethods, (value) => (settings!.deinterlace_method = value))}
					</SettingRow>
					<SettingRow label="Deinterlace double rate" hint="Keep both fields as frames: smoother motion, twice the output frame count.">
						<Switch bare label="Deinterlace double rate" checked={settings.deinterlace_double_rate} onCheckedChange={(checked) => (settings!.deinterlace_double_rate = checked)} />
					</SettingRow>
				{/if}
				<SettingRow
					label="Per-codec encoding"
					hint="Each output codec has its own preset and CRF, like Jellyfin. Leave a preset on “inherit” to use the preset above."
					stack
				>
					<div class="w-full space-y-2">
						{#each codecRows as row (row.key)}
							<div class="grid grid-cols-[4.5rem_minmax(0,1fr)_7rem] items-center gap-3 sm:grid-cols-[6rem_minmax(0,20rem)_7rem]">
								<p class="text-sm font-medium text-foreground">{row.label}</p>
								<Select
									aria-label={`${row.label} preset`}
									value={row.preset()}
									options={[{ value: '', label: 'Inherit' }, ...presets.map((p) => ({ value: p, label: p }))]}
									onValueChange={(value) => row.setPreset(value)}
								/>
								<input
									type="number"
									class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none"
									aria-label={`${row.label} CRF`}
									min="0"
									max="51"
									value={String(row.crf())}
									oninput={(e) => row.setCRF(numberValue(e))}
								/>
							</div>
						{/each}
					</div>
				</SettingRow>
				<SettingRow label="Quality ladder" hint={t('settings.groups.transcoding.ladderHint')} stack>
					<div class="w-full overflow-x-auto">
						<table class="w-full min-w-[28rem] text-sm">
							<thead>
								<tr class="text-left font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
									<th class="pb-2 font-normal">Name</th>
									<th class="pb-2 font-normal">Max w</th>
									<th class="pb-2 font-normal">Max h</th>
									<th class="pb-2 font-normal">kbps</th>
									<th class="w-9"></th>
								</tr>
							</thead>
							<tbody>
								{#each settings.qualities as quality, index (index)}
									<tr class="border-t border-hairline">
										<td class="py-1.5 pr-2"><input class="h-9 w-full rounded-md border px-3 font-mono text-xs text-foreground outline-none" aria-label="Quality name" value={quality.name} oninput={(e) => setQualityField(index, 'name', e.currentTarget.value)} /></td>
										<td class="py-1.5 pr-2"><input class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none" aria-label="Max width" type="number" value={String(quality.max_width ?? 0)} oninput={(e) => setQualityField(index, 'max_width', numberValue(e))} /></td>
										<td class="py-1.5 pr-2"><input class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none" aria-label="Max height" type="number" value={String(quality.max_height ?? 0)} oninput={(e) => setQualityField(index, 'max_height', numberValue(e))} /></td>
										<td class="py-1.5 pr-2"><input class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none" aria-label="Bitrate kbps" type="number" value={String(quality.bitrate_kbps ?? 0)} oninput={(e) => setQualityField(index, 'bitrate_kbps', numberValue(e))} /></td>
										<td class="py-1.5 text-right">
											<button type="button" class="inline-flex size-9 items-center justify-center rounded-md text-muted hover:bg-surface-hover hover:text-danger" aria-label="Remove quality {quality.name}" onclick={() => removeQuality(index)}>
												<Trash2 class="size-4" />
											</button>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
					<Button variant="tonal" size="field" onclick={addQuality}><Plus class="size-4" /> Add rung</Button>
				</SettingRow>
			</SettingsGroup>
		{/if}

		{#if sectionMatches('hardware', 'acceleration', 'gpu', 'vaapi', 'nvenc', 'qsv', 'decode')}
			<SettingsGroup
				id="hardware"
				title="Hardware acceleration"
				description="Automatic GPU encode and decode, guarded by the server probe."
				open={hardwareActive || !!normalizedSearch}
			>
				{#snippet aside()}
					<Badge tone={hardwareActive ? 'accent' : 'neutral'}>{hardwareActive ? settings?.hardware_acceleration.toUpperCase() : 'software'}</Badge>
				{/snippet}
				<SettingRow label="Backend">
					{@render select('Backend', settings.hardware_acceleration, probedHardwareOptions, (value) => (settings!.hardware_acceleration = value))}
				</SettingRow>
				{#if settings.hardware_acceleration !== 'none'}
					<div class="flex flex-wrap items-center justify-between gap-3 bg-accent/5 py-3">
						<p class="text-xs leading-5 text-muted">A failed runtime attempt falls back to software and is reported on the session.</p>
						<Button variant="ghost" size="field" onclick={stageSoftware}>Use software</Button>
					</div>
				{/if}
				{#if settings.hardware_acceleration === 'vaapi'}
					<SettingRow label="VA-API device" hint="VA-API render node (default /dev/dri/renderD128). Other backends pick their own device.">
						{@render textInput('VA-API device', settings.hardware_device, (v) => (settings!.hardware_device = v))}
					</SettingRow>
				{/if}
				<SettingRow label="Use hardware encoding" hint="Encode with the selected backend when its probe passes; a failed attempt falls back to software visibly.">
					<Switch bare label="Use hardware encoding" checked={settings.hardware_encode} onCheckedChange={(checked) => (settings!.hardware_encode = checked)} />
				</SettingRow>
				{#if settings.hardware_acceleration === 'qsv'}
					<SettingRow label="Intel low-power encoder" hint="Use QSV's low-power H.264/HEVC encoder (experimental; many mfx version and rate-control limitations).">
						<Switch bare label="Intel low-power encoder" checked={settings.hardware_low_power} onCheckedChange={(checked) => (settings!.hardware_low_power = checked)} />
					</SettingRow>
				{/if}
				<SettingRow label="Hardware decoding" stack>
					<div class="flex flex-wrap gap-2" role="group" aria-label="Hardware decoding">
						{#each decodeCodecs as codec (codec)}
							{@const on = settings.hardware_decode_codecs.includes(codec)}
							<label class="flex h-9 cursor-pointer items-center gap-2 rounded-md px-3 font-mono text-xs transition-colors {on ? 'bg-accent/15 text-foreground' : 'bg-field text-muted hover:text-foreground'}">
								<input
									type="checkbox"
									class="accent-current"
									checked={on}
									onchange={(e) => toggleCodec(codec, (e.currentTarget as HTMLInputElement).checked)}
								/>
								{codec}
							</label>
						{/each}
					</div>
				</SettingRow>
				<SettingRow label="Decode 10-bit HEVC in hardware" hint="Off by default: some backends fail or produce wrong colours on 10-bit HEVC.">
					<Switch bare label="Decode 10-bit HEVC in hardware" checked={settings.hardware_decode_10bit_hevc} onCheckedChange={(checked) => (settings!.hardware_decode_10bit_hevc = checked)} />
				</SettingRow>
				<SettingRow label="Decode 10-bit VP9 in hardware">
					<Switch bare label="Decode 10-bit VP9 in hardware" checked={settings.hardware_decode_10bit_vp9} onCheckedChange={(checked) => (settings!.hardware_decode_10bit_vp9 = checked)} />
				</SettingRow>
			</SettingsGroup>
		{/if}

		{#if sectionMatches('hdr', 'processing', 'tone mapping', 'luminance', 'deinterlace')}
			<SettingsGroup id="processing" title="HDR & tone mapping" description="Automatic is recommended: process HDR only when the source needs it.">
				<SettingRow label="Enable tone mapping" hint="HDR sources become SDR when the chain executes; without it HDR browser playback stays unavailable.">
					<Switch bare label="Enable tone mapping" checked={settings.tone_mapping} onCheckedChange={(checked) => (settings!.tone_mapping = checked)} />
				</SettingRow>
				{#if settings.tone_mapping}
					<SettingRow label="Algorithm">
						{@render select('Algorithm', settings.tone_mapping_algorithm, toneAlgorithms, (value) => (settings!.tone_mapping_algorithm = value))}
					</SettingRow>
					<SettingRow label="When to apply">
						{@render select('When to apply', settings.tone_mapping_mode, toneModes, (value) => (settings!.tone_mapping_mode = value))}
					</SettingRow>
					<SettingRow label="Nominal peak luminance (nits)">
						{@render numberInput('Nominal peak luminance (nits)', settings.tone_mapping_peak_nits, (n) => (settings!.tone_mapping_peak_nits = n), 50, 10000)}
					</SettingRow>
				{/if}
			</SettingsGroup>
		{/if}

		{#if sectionMatches('audio', 'subtitle', 'captions', 'downmix', 'font', 'bitrate')}
			<SettingsGroup id="audio" title="Audio & subtitles" description="Prefer extraction and direct playback; burn only when a track requires it.">
				<SettingRow label="Audio bitrate (kbps)">
					{@render numberInput('Audio bitrate (kbps)', settings.audio_bitrate_kbps, (n) => (settings!.audio_bitrate_kbps = n), 32, 640)}
				</SettingRow>
				<SettingRow label="Subtitle handling">
					{@render select('Subtitle handling', settings.subtitle_mode, subtitleModes, (value) => (settings!.subtitle_mode = value as TranscodeSettings['subtitle_mode']))}
				</SettingRow>
				<SettingRow label="Variable-bitrate audio (AAC VBR)" hint="Uses ffmpeg's native VBR quality step mapped from the bitrate above instead of a fixed rate.">
					<Switch bare label="Variable-bitrate audio (AAC VBR)" checked={settings.audio_vbr} onCheckedChange={(checked) => (settings!.audio_vbr = checked)} />
				</SettingRow>
				<SettingRow label="Allow subtitle extraction on the fly" hint="Serves WebVTT for a chosen track while a file plays directly, without transcoding it.">
					<Switch bare label="Allow subtitle extraction on the fly" checked={settings.allow_subtitle_extraction !== false} onCheckedChange={(checked) => (settings!.allow_subtitle_extraction = checked)} />
				</SettingRow>
				<SettingRow label="Downmix multichannel audio" hint="More than two channels become stereo AAC.">
					<Switch bare label="Downmix multichannel audio" checked={settings.downmix_audio} onCheckedChange={(checked) => (settings!.downmix_audio = checked)} />
				</SettingRow>
				{#if settings.downmix_audio}
					<SettingRow label="Stereo downmix algorithm">
						{@render select('Stereo downmix algorithm', settings.downmix_stereo_algorithm, downmixAlgorithms, (value) => (settings!.downmix_stereo_algorithm = value))}
					</SettingRow>
					<SettingRow label="Downmix gain (linear)" hint="Applied only when downmixing; 1 leaves the level untouched.">
						{@render numberInput('Downmix gain (linear)', settings.downmix_audio_boost, (n) => (settings!.downmix_audio_boost = n), 0.5, 8, 0.5)}
					</SettingRow>
				{/if}
				<SettingRow label="Enable fallback fonts" hint="Use the font directory and forced family below when a burned-in subtitle needs a font.">
					<Switch bare label="Enable fallback fonts" checked={settings.fallback_font_enabled !== false} onCheckedChange={(checked) => (settings!.fallback_font_enabled = checked)} />
				</SettingRow>
				<SettingRow label="Burn-in font directory" hint="Searched first when a burned-in subtitle needs a font.">
					{@render textInput('Burn-in font directory', settings.fallback_font_path, (v) => (settings!.fallback_font_path = v), 'system font path')}
				</SettingRow>
				<SettingRow label="Forced font family">
					{@render textInput('Forced font family', settings.fallback_font_name, (v) => (settings!.fallback_font_name = v), 'e.g. DejaVu Sans')}
				</SettingRow>
			</SettingsGroup>
		{/if}

		{#if sectionMatches('performance', 'resources', 'storage', 'cache', 'queue', 'path', 'ffmpeg', 'threads', 'bitrate')}
			<SettingsGroup
				id="resources"
				title="Performance, resources & storage"
				description="Capacity limits, cache, process paths and temporary storage."
				open={customResourcesActive || !!normalizedSearch}
			>
				{#snippet aside()}
					<Badge tone={customResourcesActive ? 'accent' : 'neutral'}>{customResourcesActive ? 'configured' : 'automatic'}</Badge>
				{/snippet}
				<SettingRow label="Transcoding thread count (0 = auto)">
					{@render numberInput('Transcoding thread count (0 = auto)', settings.thread_count, (n) => (settings!.thread_count = n), 0, 64)}
				</SettingRow>
				<SettingRow label="Max muxing queue size (packets)" hint="Buffers output packets so a slow client cannot abort the muxer.">
					{@render numberInput('Max muxing queue size (packets)', settings.max_muxing_queue_size, (n) => (settings!.max_muxing_queue_size = n), 128, 65536)}
				</SettingRow>
				<SettingRow label="Transcoding temporary path" hint="Artifacts move to <path>/lain-transcode/<session>/; the JSON index stays in the data dir.">
					{@render textInput('Transcoding temporary path', settings.transcode_temp_path, (v) => (settings!.transcode_temp_path = v), 'data dir (default)')}
				</SettingRow>
				<SettingRow label="Remote client bitrate limit (kbps, 0 = unlimited)" hint="Server-wide ceiling; the tighter of this and a user's own limit wins.">
					{@render numberInput('Remote client bitrate limit (kbps, 0 = unlimited)', settings.remote_bitrate_limit_kbps, (n) => (settings!.remote_bitrate_limit_kbps = n), 0, 1000000)}
				</SettingRow>
				<SettingRow label={`Cache budget (bytes — ${formatBytes(settings.cache_bytes)})`}>
					{@render numberInput('Cache budget (bytes)', settings.cache_bytes, (n) => (settings!.cache_bytes = n), undefined, undefined, undefined, true)}
				</SettingRow>
				<SettingRow label="Queue size">
					{@render numberInput('Queue size', settings.queue_size, (n) => (settings!.queue_size = n), 1, 64)}
				</SettingRow>
				<SettingRow label="Max concurrent sessions">
					{@render numberInput('Max concurrent sessions', settings.max_concurrent, (n) => (settings!.max_concurrent = n), 1, 16)}
				</SettingRow>
				<SettingRow label="ffmpeg path">
					{@render textInput('ffmpeg path', settings.ffmpeg_path, (v) => (settings!.ffmpeg_path = v), 'system PATH')}
				</SettingRow>
				<SettingRow label="ffprobe path">
					{@render textInput('ffprobe path', settings.ffprobe_path, (v) => (settings!.ffprobe_path = v), 'system PATH')}
				</SettingRow>
			</SettingsGroup>
		{/if}

		{#if searchMatches('active sessions', 'session', 'running', 'queue', 'fps', 'encoder')}
			<SettingsGroup id="sessions" title="Active sessions" description={t('settings.groups.transcoding.sessions')}>
				{#snippet aside()}
					<Button variant="tonal" size="field" onclick={() => void refreshSessions()}>Refresh</Button>
				{/snippet}
				{#if sessions.length === 0}
					<p class="py-4 text-sm text-muted">Nothing is being prepared right now.</p>
				{:else}
					<ul class="settings-items">
						{#each sessions as session (session.session)}
							<li class="flex flex-wrap items-center gap-3 py-2.5 text-sm">
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
										<Button variant="ghost" size="field" onclick={() => void cancelSession(session.session)}>
											Stop
										</Button>
									{/if}
								</div>
							</li>
						{/each}
					</ul>
				{/if}
			</SettingsGroup>
		{/if}

		{#if normalizedSearch && !hasSearchResults}
			<div class="rounded-card bg-panel p-10 text-center">
				<Search class="mx-auto size-5 text-muted" />
				<p class="mt-3 text-sm font-medium text-foreground">No setting matches “{searchQuery.trim()}”</p>
				<p class="mt-1 text-xs text-muted">Try a feature, codec, resource or delivery term.</p>
			</div>
		{/if}

		<StagedBar count={changeCount} {saving} onapply={() => void save()} ondiscard={resetChanges} />
	{/if}
</div>
