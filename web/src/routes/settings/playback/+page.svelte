<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import Clapperboard from '@lucide/svelte/icons/clapperboard';
	import Cpu from '@lucide/svelte/icons/cpu';
	import Save from '@lucide/svelte/icons/save';
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
	import Input from '$lib/components/primitives/Input.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let loading = $state(true);
	let saving = $state(false);
	let error = $state<string | null>(null);
	let settings = $state<TranscodeSettings | null>(null);
	let capabilities = $state<TranscodeCapabilities | null>(null);
	let sessions = $state<TranscodeStatus[]>([]);
	let sessionBusy = $state<string | null>(null);
	let refreshTimer: ReturnType<typeof setInterval> | null = null;

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
			settings = response.settings;
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
			settings = response.settings;
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

<div class="space-y-6">
	{#if loading}
		<div class="h-40 animate-pulse rounded-card bg-surface-active/50"></div>
	{:else if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if settings}
		<div class="flex flex-wrap items-center justify-between gap-3">
			<p class="max-w-2xl text-sm text-muted">
				Runtime transcoding policy. Changes apply to new sessions without a restart; hardware
				acceleration is opt-in and only used when its probe passes — otherwise the server falls
				back to software and reports it on the session.
			</p>
			<Button loading={saving} onclick={() => void save()}>
				<Save class="size-4" /> Save
			</Button>
		</div>

		{#if capabilities}
			<section class="rounded-card border border-line bg-surface/40 p-4">
				<div class="flex items-center gap-2">
					<Cpu class="size-4 text-muted" />
					<h2 class="text-sm font-semibold text-foreground">Probed capabilities</h2>
				</div>
				<p class="mt-1 text-xs text-muted">ffmpeg: {capabilities.ffmpeg || 'system PATH'}</p>
				<div class="mt-3 flex flex-wrap gap-1.5">
					<Badge tone={capabilities.tone_mapping ? 'success' : 'danger'}>
						tone mapping {capabilities.tone_mapping ? 'available' : 'unavailable'}
					</Badge>
					<Badge tone={capabilities.tone_mapping_bt2390 ? 'success' : 'neutral'}>
						BT.2390 {capabilities.tone_mapping_bt2390 ? 'available' : 'fallback to hable'}
					</Badge>
					{#each Object.entries(capabilities.hardware) as [backend, ok] (backend)}
						<Badge tone={ok ? 'success' : 'danger'}>{backend} {ok ? 'ready' : 'probe failed'}</Badge>
					{/each}
					{#each capabilities.encoders as encoder (encoder)}
						<Badge>{encoder}</Badge>
					{/each}
				</div>
			</section>
		{/if}

		<!-- Delivery -->
		<section class="space-y-4 rounded-card border border-line bg-surface/40 p-4">
			<div class="flex items-center gap-2">
				<Clapperboard class="size-4 text-muted" />
				<h2 class="text-sm font-semibold text-foreground">Delivery</h2>
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

		<!-- Encoding -->
		<section class="space-y-4 rounded-card border border-line bg-surface/40 p-4">
			<h2 class="text-sm font-semibold text-foreground">Encoding</h2>
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

		<!-- Hardware -->
		<section class="space-y-4 rounded-card border border-line bg-surface/40 p-4">
			<h2 class="text-sm font-semibold text-foreground">Hardware acceleration</h2>
			<Select
				label="Backend"
				value={settings.hardware_acceleration}
				options={hardwareOptions}
				onValueChange={(value) => (settings!.hardware_acceleration = value)}
			/>
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
		</section>

		<!-- HDR -->
		<section class="space-y-4 rounded-card border border-line bg-surface/40 p-4">
			<h2 class="text-sm font-semibold text-foreground">HDR & tone mapping</h2>
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

		<!-- Audio & subtitles -->
		<section class="space-y-4 rounded-card border border-line bg-surface/40 p-4">
			<h2 class="text-sm font-semibold text-foreground">Audio & subtitles</h2>
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

		<!-- Resources & paths -->
		<section class="space-y-4 rounded-card border border-line bg-surface/40 p-4">
			<h2 class="text-sm font-semibold text-foreground">Performance, resources & storage</h2>
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
		</section>

		<!-- Sessions -->
		<section class="space-y-3 rounded-card border border-line bg-surface/40 p-4">
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

		<div class="flex justify-end">
			<Button loading={saving} onclick={() => void save()}>
				<Save class="size-4" /> Save
			</Button>
		</div>
	{/if}
</div>
