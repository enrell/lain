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
	import Segmented from '$lib/components/primitives/Segmented.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import {
		applySafeAutomaticPolicy,
		availableHardwareBackends,
		cloneTranscodeSettings,
		engineChoices,
		isAutomatic
	} from '$lib/settings/playback-policy';
	import { page } from '$app/state';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import StagedBar from '$lib/components/settings/StagedBar.svelte';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { i18n, t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes } from '$lib/utilities/format';

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

	const readyHardware = $derived(availableHardwareBackends(capabilities));
	const activeSessions = $derived(
		sessions.filter((session) => session.state === 'queued' || session.state === 'running').length
	);
	const normalizedSearch = $derived(searchQuery.trim().toLowerCase());
	const hardwareActive = $derived(!!settings && settings.hardware_acceleration !== 'none');
	const engines = $derived(engineChoices(settings?.hardware_acceleration ?? 'none', readyHardware));
	const customResourcesActive = $derived(
		!!settings &&
			(!!settings.transcode_temp_path ||
				settings.remote_bitrate_limit_kbps > 0 ||
				!!settings.ffmpeg_path ||
				!!settings.ffprobe_path ||
				settings.thread_count > 0)
	);

	const presets = ['ultrafast', 'superfast', 'veryfast', 'faster', 'fast', 'medium', 'slow', 'slower', 'veryslow'];
	const decodeCodecs = ['h264', 'hevc', 'mpeg2', 'vc1', 'vp8', 'vp9', 'av1'];
	// Backend names are product names: they stay as they are in every language.
	const BACKENDS: Record<string, string> = {
		vaapi: 'VAAPI',
		nvenc: 'NVENC',
		qsv: 'QSV',
		amf: 'AMF',
		v4l2m2m: 'V4L2',
		videotoolbox: 'VideoToolbox',
		rkmpp: 'RKMPP'
	};
	const BACKEND_NAMES: Record<string, string> = {
		vaapi: 'VAAPI',
		nvenc: 'NVIDIA NVENC',
		qsv: 'Intel Quick Sync',
		amf: 'AMD AMF',
		v4l2m2m: 'V4L2 mem2mem',
		videotoolbox: 'VideoToolbox (macOS)',
		rkmpp: 'Rockchip RKMPP'
	};
	function backendLabel(id: string): string {
		return id === 'none' ? t('settings.transcode.software') : (BACKENDS[id] ?? id.toUpperCase());
	}

	const deliveries = $derived([
		{ value: 'hls', label: t('settings.transcode.delivery.hls') },
		{ value: 'progressive', label: t('settings.transcode.delivery.progressive') }
	]);
	const containers = $derived([
		{ value: 'fmp4', label: t('settings.transcode.delivery.fmp4') },
		{ value: 'mpegts', label: t('settings.transcode.delivery.mpegts') }
	]);
	const toneAlgorithms = $derived([
		{ value: 'bt2390', label: t('settings.transcode.hdr.bt2390') },
		{ value: 'hable', label: 'Hable' },
		{ value: 'reinhard', label: 'Reinhard' },
		{ value: 'mobius', label: 'Mobius' },
		{ value: 'clip', label: 'Clip' },
		{ value: 'linear', label: 'Linear' }
	]);
	const toneModes = $derived([
		{ value: 'auto', label: t('settings.transcode.hdr.modeAuto') },
		{ value: 'always', label: t('settings.transcode.hdr.modeAlways') },
		{ value: 'never', label: t('settings.transcode.hdr.modeNever') }
	]);
	const hardwareOptions = $derived([
		{ value: 'none', label: t('settings.transcode.hardware.none') },
		...Object.keys(BACKEND_NAMES).map((value) => {
			const off = capabilities !== null && capabilities.hardware[value] !== true;
			return {
				value,
				disabled: off,
				label: off ? t('settings.transcode.hardware.unavailable', { name: BACKEND_NAMES[value] }) : BACKEND_NAMES[value]
			};
		})
	]);
	const subtitleModes = $derived([
		{ value: 'auto', label: t('settings.transcode.audio.subtitleAuto') },
		{ value: 'extract', label: t('settings.transcode.audio.subtitleExtract') },
		{ value: 'burn', label: t('settings.transcode.audio.subtitleBurn') },
		{ value: 'off', label: t('settings.transcode.audio.subtitleOff') }
	]);
	const deinterlaceMethods = $derived([
		{ value: 'yadif', label: 'yadif' },
		{ value: 'bwdif', label: t('settings.transcode.encoding.bwdif') }
	]);
	const downmixAlgorithms = $derived([
		{ value: 'none', label: t('settings.transcode.audio.downmixNone') },
		{ value: 'nightmode', label: t('settings.transcode.audio.downmixNight') }
	]);

	// The per-codec encoder settings as small accessors, so the markup stays
	// a plain list instead of three near-identical blocks.
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
			// when it is what Auto would stage from this probe.
			if (!CUSTOM_ANCHORS.includes(page.url.hash.slice(1)))
				view = isAutomatic(response.settings, response.capabilities) ? 'auto' : 'custom';
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
			toasts.success(t('settings.transcode.saved'));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.transcode.saveFailed')));
		} finally {
			saving = false;
		}
	}

	async function cancelSession(session: string): Promise<void> {
		sessionBusy = session;
		try {
			await api.transcodeAdmin.cancelSession(session);
			sessions = sessions.filter((s) => s.session !== session);
			toasts.success(t('settings.transcode.sessions.stopped'));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.transcode.sessions.stopFailed')));
		} finally {
			sessionBusy = null;
		}
	}

	function addQuality(): void {
		if (!settings) return;
		settings.qualities = [...settings.qualities, { name: 'custom', max_width: 1920, max_height: 1080, bitrate_kbps: 6000 }];
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
		view = isAutomatic(persistedSettings, capabilities) ? 'auto' : 'custom';
		toasts.info(t('settings.transcode.reset'));
	}

	function stageHardware(backend: string): void {
		if (!settings || !readyHardware.includes(backend)) return;
		settings.hardware_acceleration = backend;
		settings.hardware_encode = true;
	}

	function stageSoftware(): void {
		if (!settings) return;
		settings.hardware_acceleration = 'none';
		settings.hardware_encode = false;
		settings.hardware_low_power = false;
	}

	/*
	 * Auto stages what the probe allows and shows what it chose in one line;
	 * Custom lists every section, each closed to a line of its current values
	 * until opened, so the page reads top to bottom in one screen. A search or
	 * a Ctrl+K jump to a section opens it.
	 */
	const CUSTOM_ANCHORS = ['delivery', 'encoding', 'hardware', 'processing', 'audio', 'resources'];
	let view = $state<'auto' | 'custom'>('auto');
	// Set from the start: a group decides once whether it is collapsible.
	let opened = $state<Record<string, boolean>>({
		delivery: false,
		encoding: false,
		hardware: false,
		processing: false,
		audio: false,
		resources: false,
		capabilities: false
	});
	$effect(() => {
		const hash = page.url.hash.slice(1);
		if (CUSTOM_ANCHORS.includes(hash)) {
			view = 'custom';
			opened[hash] = true;
		}
	});
	$effect(() => {
		if (normalizedSearch) for (const id of [...CUSTOM_ANCHORS, 'capabilities']) opened[id] = true;
	});
	function setView(next: 'auto' | 'custom'): void {
		view = next;
		if (next === 'auto' && settings) settings = applySafeAutomaticPolicy(settings, capabilities);
	}

	const changeCount = $derived.by(() => {
		if (!settings || !persistedSettings) return 0;
		const a = settings as unknown as Record<string, unknown>;
		const b = persistedSettings as unknown as Record<string, unknown>;
		return Object.keys({ ...a, ...b }).filter((k) => JSON.stringify(a[k]) !== JSON.stringify(b[k])).length;
	});

	// Search terms per section: the translated title and its English
	// technical words (codec and flag names read the same everywhere).
	const KEYWORDS: Record<string, string> = {
		capabilities: 'diagnostics capabilities ffmpeg encoder probe',
		delivery: 'delivery stream hls segment timeout throttle progressive',
		encoding: 'encoding quality codec crf preset hevc av1 deinterlace ladder h264',
		hardware: 'hardware acceleration gpu vaapi nvenc qsv decode',
		processing: 'hdr processing tone mapping luminance',
		audio: 'audio subtitle captions downmix font bitrate',
		resources: 'performance resources storage cache queue path ffmpeg threads bitrate',
		sessions: 'active sessions session running queue fps'
	};
	function shows(id: string, title: string, always = false): boolean {
		if (!normalizedSearch) return always || view === 'custom';
		return `${title} ${KEYWORDS[id] ?? ''}`.toLowerCase().includes(normalizedSearch);
	}
	const sectionTitles = $derived({
		capabilities: t('settings.transcode.probe.title'),
		delivery: t('settings.transcode.delivery.title'),
		encoding: t('settings.transcode.encoding.title'),
		hardware: t('settings.transcode.hardware.title'),
		processing: t('settings.transcode.hdr.title'),
		audio: t('settings.transcode.audio.title'),
		resources: t('settings.transcode.resources.title'),
		sessions: t('settings.transcode.sessions.title')
	});
	const anyShown = $derived(
		Object.entries(sectionTitles).some(([id, title]) => shows(id, title, id === 'capabilities' || id === 'sessions'))
	);

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

	// One line of each closed section: what is in force, in the words the
	// rows below use.
	const summaries = $derived.by(() => {
		const s = settings;
		if (!s) return {} as Record<string, string>;
		const list = (parts: string[]) => parts.filter(Boolean).join(' · ');
		const ladder = s.qualities.map((q) => q.name);
		return {
			delivery: list([
				s.default_delivery === 'hls' ? t('settings.transcode.delivery.sumHls', { seconds: s.hls_segment_seconds }) : t('settings.transcode.delivery.sumProgressive'),
				s.default_delivery === 'hls' ? (s.hls_segment_container === 'fmp4' ? 'fMP4' : 'MPEG-TS') : '',
				s.throttle ? t('settings.transcode.delivery.sumThrottle') : '',
				t('settings.transcode.delivery.sumIdle', { seconds: s.idle_timeout_sec })
			]),
			encoding: list([
				`H.264 ${s.encoder_preset}`,
				`CRF ${s.crf}`,
				s.allow_hevc ? 'HEVC' : '',
				s.allow_av1 ? 'AV1' : '',
				ladder.length
					? t('settings.transcode.encoding.sumLadder', { count: ladder.length, range: ladder.length > 1 ? `${ladder[0]}–${ladder[ladder.length - 1]}` : ladder[0] })
					: t('settings.transcode.encoding.sumNoLadder')
			]),
			hardware: hardwareActive
				? list([
						t('settings.transcode.hardware.sumEncode', { name: backendLabel(s.hardware_acceleration) }),
						s.hardware_decode_codecs.length ? t('settings.transcode.hardware.sumDecode', { codecs: i18n.formatList(s.hardware_decode_codecs) }) : ''
					])
				: readyHardware.length
					? t('settings.transcode.hardware.sumOff')
					: t('settings.transcode.hardware.sumNone'),
			processing: !s.tone_mapping
				? t('settings.transcode.hdr.sumOff')
				: list([
						s.tone_mapping_mode === 'auto' ? t('settings.transcode.hdr.sumAuto') : s.tone_mapping_mode === 'always' ? t('settings.transcode.hdr.sumAlways') : t('settings.transcode.hdr.sumNever'),
						// What runs, not what was asked for: BT.2390 without libplacebo is Hable.
						s.tone_mapping_algorithm === 'bt2390'
							? capabilities?.tone_mapping_bt2390
								? 'BT.2390'
								: t('settings.transcode.hdr.sumFallback')
							: s.tone_mapping_algorithm.replace(/^./, (c) => c.toUpperCase())
					]),
			audio: list([
				t('settings.transcode.audio.sumBitrate', { kbps: s.audio_bitrate_kbps }),
				s.downmix_audio ? t('settings.transcode.audio.sumDownmix') : '',
				t(`settings.transcode.audio.sumSubtitles.${s.subtitle_mode}`)
			]),
			resources: customResourcesActive
				? t('settings.transcode.resources.sumCustom', { sessions: s.max_concurrent, cache: formatBytes(s.cache_bytes) })
				: t('settings.transcode.resources.sumAuto', { sessions: s.max_concurrent, cache: formatBytes(s.cache_bytes) })
		} as Record<string, string>;
	});

	// What Auto chose, in one line (Custom shows it per section instead);
	// the engine and HDR have rows of their own below it.
	const autoLine = $derived(
		settings
			? [summaries.delivery?.split(' · ')[0], `H.264 ${settings.encoder_preset} · CRF ${settings.crf}`, t(`settings.transcode.audio.sumSubtitles.${settings.subtitle_mode}`)]
					.filter(Boolean)
					.join(' · ')
			: ''
	);
	// Another engine than the one Auto picks is a choice of one's own.
	function pickEngine(id: string): void {
		if (id === 'none') stageSoftware();
		else stageHardware(id);
		if (view === 'auto' && id !== (readyHardware[0] ?? 'none')) view = 'custom';
	}

	// "/" jumps to the setting search, as in most developer tools.
	let searchInput = $state<HTMLInputElement>();
	usePageKeys(() => [{ keys: ['/'], label: t('pageKeys.settingsFilter') }]);

	function onSlash(e: KeyboardEvent): void {
		const target = e.target as HTMLElement | null;
		if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return;
		if (target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))) return;
		e.preventDefault();
		searchInput?.focus();
	}
</script>

<svelte:window onkeydown={onSlash} />

<svelte:head><title>{t('settings.transcode.pageTitle')}</title></svelte:head>

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
	{:else if settings && capabilities}
		<div class="relative mb-10">
			<Search class="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted" />
			<input
				bind:this={searchInput}
				bind:value={searchQuery}
				aria-label={t('settings.transcode.filterLabel')}
				placeholder={t('settings.transcode.filter')}
				class="h-10 w-full rounded-md border ps-9 pe-10 text-sm text-foreground placeholder:text-muted/60 focus:outline-none"
			/>
			<kbd class="pointer-events-none absolute end-3 top-1/2 -translate-y-1/2">/</kbd>
		</div>

		{#if !normalizedSearch}
			<!-- What this machine can do, and who decides the rest. Each row is a
			     fact about the server or one choice; nothing here repeats a
			     section below. -->
			<SettingsGroup id="mode" title={t('settings.transcode.machine.title')} description={t('settings.transcode.machine.description')}>
				<SettingRow
					id="policy"
					label={t('settings.transcode.policy.label')}
					hint={view === 'auto' ? t('settings.transcode.policy.autoHint') : t('settings.transcode.policy.customHint')}
				>
					<Segmented
						label={t('settings.transcode.policy.label')}
						value={view}
						options={[
							{ value: 'auto', label: t('settings.transcode.policy.auto') },
							{ value: 'custom', label: t('settings.transcode.policy.custom') }
						]}
						onchange={setView}
					/>
				</SettingRow>
				{#if view === 'auto'}
					<SettingRow label={t('settings.transcode.policy.chose')} hint={autoLine} />
				{/if}
				{#if engines.length > 1}
					<SettingRow
						label={t('settings.transcode.engine.label')}
						hint={hardwareActive ? t('settings.transcode.engine.hintHardware') : t('settings.transcode.engine.hintSoftware')}
					>
						<!-- Only what the probe passed (and the backend in force, so the
						     control still shows it after a probe stops passing it). -->
						<Segmented
							label={t('settings.transcode.engine.label')}
							value={hardwareActive ? settings.hardware_acceleration : 'none'}
							options={engines.map((id) => ({
								value: id,
								label: backendLabel(id),
								disabled: id !== 'none' && !readyHardware.includes(id),
								title: id !== 'none' && !readyHardware.includes(id) ? t('settings.transcode.engine.failedProbe') : undefined
							}))}
							onchange={pickEngine}
						/>
					</SettingRow>
				{:else}
					<!-- Nothing to choose: a one-segment control would offer a choice
					     that does not exist. Say what happens instead. -->
					<SettingRow label={t('settings.transcode.engine.label')} hint={t('settings.transcode.engine.noHardware')}>
						<span class="font-mono text-xs uppercase tracking-[0.14em] text-foreground">{t('settings.transcode.software')}</span>
					</SettingRow>
				{/if}
				<SettingRow
					label={t('settings.transcode.hdrRow.label')}
					hint={capabilities.tone_mapping ? t('settings.transcode.hdrRow.ready') : t('settings.transcode.hdrRow.missing')}
				>
					<span class="font-mono text-xs uppercase tracking-[0.14em] {capabilities.tone_mapping ? 'text-foreground' : 'text-warning'}">
						{capabilities.tone_mapping
							? capabilities.tone_mapping_bt2390
								? 'BT.2390'
								: 'Hable'
							: t('settings.transcode.hdrRow.none')}
					</span>
				</SettingRow>
			</SettingsGroup>
		{/if}

		{#if shows('delivery', sectionTitles.delivery)}
			<SettingsGroup id="delivery" title={sectionTitles.delivery} description={summaries.delivery} bind:open={opened.delivery}>
				<SettingRow label={t('settings.transcode.delivery.default')}>
					{@render select(t('settings.transcode.delivery.default'), settings.default_delivery, deliveries, (value) => (settings!.default_delivery = value as TranscodeSettings['default_delivery']))}
				</SettingRow>
				{#if settings.default_delivery === 'hls'}
					<SettingRow label={t('settings.transcode.delivery.container')}>
						{@render select(t('settings.transcode.delivery.container'), settings.hls_segment_container, containers, (value) => (settings!.hls_segment_container = value))}
					</SettingRow>
					<SettingRow label={t('settings.transcode.delivery.segment')}>
						{@render numberInput(t('settings.transcode.delivery.segment'), settings.hls_segment_seconds, (n) => (settings!.hls_segment_seconds = n), 1, 30)}
					</SettingRow>
				{/if}
				<SettingRow label={t('settings.transcode.delivery.idle')} hint={t('settings.transcode.delivery.idleHint')}>
					{@render numberInput(t('settings.transcode.delivery.idle'), settings.idle_timeout_sec, (n) => (settings!.idle_timeout_sec = n), 15, 3600)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.delivery.throttle')} hint={t('settings.transcode.delivery.throttleHint')}>
					<Switch bare label={t('settings.transcode.delivery.throttle')} checked={settings.throttle} onCheckedChange={(checked) => (settings!.throttle = checked)} />
				</SettingRow>
				{#if settings.throttle}
					<SettingRow label={t('settings.transcode.delivery.ahead')} hint={t('settings.transcode.delivery.aheadHint')}>
						{@render numberInput(t('settings.transcode.delivery.ahead'), settings.throttle_ahead_sec, (n) => (settings!.throttle_ahead_sec = n), 5, 600)}
					</SettingRow>
				{/if}
				<SettingRow label={t('settings.transcode.delivery.deletion')} hint={t('settings.transcode.delivery.deletionHint')}>
					<Switch bare label={t('settings.transcode.delivery.deletion')} checked={settings.segment_deletion} onCheckedChange={(checked) => (settings!.segment_deletion = checked)} />
				</SettingRow>
				{#if settings.segment_deletion}
					<SettingRow label={t('settings.transcode.delivery.keep')}>
						{@render numberInput(t('settings.transcode.delivery.keep'), settings.segment_keep_sec, (n) => (settings!.segment_keep_sec = n), 0, 3600)}
					</SettingRow>
				{/if}
			</SettingsGroup>
		{/if}

		{#if shows('encoding', sectionTitles.encoding)}
			<SettingsGroup id="encoding" title={sectionTitles.encoding} description={summaries.encoding} bind:open={opened.encoding}>
				<SettingRow label={t('settings.transcode.encoding.preset')} hint={t('settings.transcode.encoding.presetHint')}>
					{@render select(t('settings.transcode.encoding.preset'), settings.encoder_preset, presets.map((p) => ({ value: p, label: p })), (value) => (settings!.encoder_preset = value))}
				</SettingRow>
				<SettingRow label={t('settings.transcode.encoding.crf')} hint={t('settings.transcode.encoding.crfHint')}>
					{@render numberInput(t('settings.transcode.encoding.crf'), settings.crf, (n) => (settings!.crf = n), 0, 51)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.encoding.hevc')} hint={t('settings.transcode.encoding.hevcHint')}>
					<Switch bare label={t('settings.transcode.encoding.hevc')} checked={settings.allow_hevc} onCheckedChange={(checked) => (settings!.allow_hevc = checked)} />
				</SettingRow>
				<SettingRow label={t('settings.transcode.encoding.av1')} hint={t('settings.transcode.encoding.av1Hint')}>
					<Switch bare label={t('settings.transcode.encoding.av1')} checked={settings.allow_av1} onCheckedChange={(checked) => (settings!.allow_av1 = checked)} />
				</SettingRow>
				<SettingRow label={t('settings.transcode.encoding.deinterlace')}>
					<Switch bare label={t('settings.transcode.encoding.deinterlace')} checked={settings.deinterlace === 'auto'} onCheckedChange={(checked) => (settings!.deinterlace = checked ? 'auto' : 'off')} />
				</SettingRow>
				{#if settings.deinterlace === 'auto'}
					<SettingRow label={t('settings.transcode.encoding.method')}>
						{@render select(t('settings.transcode.encoding.method'), settings.deinterlace_method, deinterlaceMethods, (value) => (settings!.deinterlace_method = value))}
					</SettingRow>
					<SettingRow label={t('settings.transcode.encoding.doubleRate')} hint={t('settings.transcode.encoding.doubleRateHint')}>
						<Switch bare label={t('settings.transcode.encoding.doubleRate')} checked={settings.deinterlace_double_rate} onCheckedChange={(checked) => (settings!.deinterlace_double_rate = checked)} />
					</SettingRow>
				{/if}
				<SettingRow label={t('settings.transcode.encoding.perCodec')} hint={t('settings.transcode.encoding.perCodecHint')} stack>
					<div class="w-full space-y-2">
						{#each codecRows as row (row.key)}
							<div class="grid grid-cols-[4.5rem_minmax(0,1fr)_7rem] items-center gap-3 sm:grid-cols-[6rem_minmax(0,20rem)_7rem]">
								<p class="text-sm font-medium text-foreground">{row.label}</p>
								<Select
									aria-label={t('settings.transcode.encoding.codecPreset', { codec: row.label })}
									value={row.preset()}
									options={[{ value: '', label: t('settings.transcode.encoding.inherit') }, ...presets.map((p) => ({ value: p, label: p }))]}
									onValueChange={(value) => row.setPreset(value)}
								/>
								<input
									type="number"
									class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none"
									aria-label={t('settings.transcode.encoding.codecCrf', { codec: row.label })}
									min="0"
									max="51"
									value={String(row.crf())}
									oninput={(e) => row.setCRF(numberValue(e))}
								/>
							</div>
						{/each}
					</div>
				</SettingRow>
				<SettingRow label={t('settings.transcode.encoding.ladder')} hint={t('settings.groups.transcoding.ladderHint')} stack>
					<!-- Positioned, so the sr-only column name (absolute) scrolls inside it:
					     unpositioned, it escaped the scroller and widened a phone's page. -->
					<div class="relative w-full overflow-x-auto">
						<table class="w-full min-w-[28rem] text-sm">
							<thead>
								<tr class="text-start font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
									<th class="pb-2 text-start font-normal">{t('settings.transcode.encoding.colName')}</th>
									<th class="pb-2 text-start font-normal">{t('settings.transcode.encoding.colWidth')}</th>
									<th class="pb-2 text-start font-normal">{t('settings.transcode.encoding.colHeight')}</th>
									<th class="pb-2 text-start font-normal">kbps</th>
									<th class="w-9"><span class="sr-only">{t('settings.transcode.encoding.colRemove')}</span></th>
								</tr>
							</thead>
							<tbody>
								{#each settings.qualities as quality, index (index)}
									<tr class="border-t border-hairline">
										<td class="py-1.5 pe-2"><input class="h-9 w-full rounded-md border px-3 font-mono text-xs text-foreground outline-none" aria-label={t('settings.transcode.encoding.colName')} value={quality.name} oninput={(e) => setQualityField(index, 'name', e.currentTarget.value)} /></td>
										<td class="py-1.5 pe-2"><input class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none" aria-label={t('settings.transcode.encoding.colWidth')} type="number" value={String(quality.max_width ?? 0)} oninput={(e) => setQualityField(index, 'max_width', numberValue(e))} /></td>
										<td class="py-1.5 pe-2"><input class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none" aria-label={t('settings.transcode.encoding.colHeight')} type="number" value={String(quality.max_height ?? 0)} oninput={(e) => setQualityField(index, 'max_height', numberValue(e))} /></td>
										<td class="py-1.5 pe-2"><input class="h-9 w-full rounded-md border px-3 text-end font-mono text-xs text-foreground outline-none" aria-label="kbps" type="number" value={String(quality.bitrate_kbps ?? 0)} oninput={(e) => setQualityField(index, 'bitrate_kbps', numberValue(e))} /></td>
										<td class="py-1.5 text-end">
											<button type="button" class="inline-flex size-9 items-center justify-center rounded-md text-muted hover:bg-surface-hover hover:text-danger" aria-label={t('settings.transcode.encoding.removeRung', { name: quality.name })} onclick={() => removeQuality(index)}>
												<Trash2 class="size-4" />
											</button>
										</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
					<Button variant="tonal" size="field" onclick={addQuality}><Plus class="size-4" /> {t('settings.transcode.encoding.addRung')}</Button>
				</SettingRow>
			</SettingsGroup>
		{/if}

		{#if shows('hardware', sectionTitles.hardware)}
			<SettingsGroup id="hardware" title={sectionTitles.hardware} description={summaries.hardware} bind:open={opened.hardware}>
				{#if readyHardware.length === 0 && !hardwareActive}
					<!-- Nothing passed the probe: no backend to pick, no decoder to
					     tune. The section says so instead of offering dead controls. -->
					<p class="py-4 text-sm text-muted">{t('settings.transcode.hardware.nothing')}</p>
				{:else}
					<SettingRow label={t('settings.transcode.hardware.backend')} hint={t('settings.transcode.hardware.backendHint')}>
						{@render select(t('settings.transcode.hardware.backend'), settings.hardware_acceleration, hardwareOptions, (value) => (value === 'none' ? stageSoftware() : (settings!.hardware_acceleration = value)))}
					</SettingRow>
					{#if settings.hardware_acceleration === 'vaapi'}
						<SettingRow label={t('settings.transcode.hardware.device')} hint={t('settings.transcode.hardware.deviceHint')}>
							{@render textInput(t('settings.transcode.hardware.device'), settings.hardware_device, (v) => (settings!.hardware_device = v))}
						</SettingRow>
					{/if}
					{#if hardwareActive}
						<SettingRow label={t('settings.transcode.hardware.encode')} hint={t('settings.transcode.hardware.encodeHint')}>
							<Switch bare label={t('settings.transcode.hardware.encode')} checked={settings.hardware_encode} onCheckedChange={(checked) => (settings!.hardware_encode = checked)} />
						</SettingRow>
						{#if settings.hardware_acceleration === 'qsv'}
							<SettingRow label={t('settings.transcode.hardware.lowPower')} hint={t('settings.transcode.hardware.lowPowerHint')}>
								<Switch bare label={t('settings.transcode.hardware.lowPower')} checked={settings.hardware_low_power} onCheckedChange={(checked) => (settings!.hardware_low_power = checked)} />
							</SettingRow>
						{/if}
						<SettingRow label={t('settings.transcode.hardware.decode')} hint={t('settings.transcode.hardware.decodeHint')} stack>
							<div class="flex flex-wrap gap-2" role="group" aria-label={t('settings.transcode.hardware.decode')}>
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
						<SettingRow label={t('settings.transcode.hardware.hevc10')} hint={t('settings.transcode.hardware.hevc10Hint')}>
							<Switch bare label={t('settings.transcode.hardware.hevc10')} checked={settings.hardware_decode_10bit_hevc} onCheckedChange={(checked) => (settings!.hardware_decode_10bit_hevc = checked)} />
						</SettingRow>
						<SettingRow label={t('settings.transcode.hardware.vp910')}>
							<Switch bare label={t('settings.transcode.hardware.vp910')} checked={settings.hardware_decode_10bit_vp9} onCheckedChange={(checked) => (settings!.hardware_decode_10bit_vp9 = checked)} />
						</SettingRow>
					{/if}
				{/if}
			</SettingsGroup>
		{/if}

		{#if shows('processing', sectionTitles.processing)}
			<SettingsGroup id="processing" title={sectionTitles.processing} description={summaries.processing} bind:open={opened.processing}>
				<SettingRow
					label={t('settings.transcode.hdr.enable')}
					hint={capabilities.tone_mapping ? t('settings.transcode.hdr.enableHint') : t('settings.transcode.hdrRow.missing')}
				>
					<Switch bare label={t('settings.transcode.hdr.enable')} checked={settings.tone_mapping} onCheckedChange={(checked) => (settings!.tone_mapping = checked)} />
				</SettingRow>
				{#if settings.tone_mapping}
					<SettingRow label={t('settings.transcode.hdr.algorithm')} hint={capabilities.tone_mapping_bt2390 ? undefined : t('settings.transcode.hdr.bt2390Missing')}>
						{@render select(t('settings.transcode.hdr.algorithm'), settings.tone_mapping_algorithm, toneAlgorithms, (value) => (settings!.tone_mapping_algorithm = value))}
					</SettingRow>
					<SettingRow label={t('settings.transcode.hdr.when')}>
						{@render select(t('settings.transcode.hdr.when'), settings.tone_mapping_mode, toneModes, (value) => (settings!.tone_mapping_mode = value))}
					</SettingRow>
					<SettingRow label={t('settings.transcode.hdr.peak')} hint={t('settings.transcode.hdr.peakHint')}>
						{@render numberInput(t('settings.transcode.hdr.peak'), settings.tone_mapping_peak_nits, (n) => (settings!.tone_mapping_peak_nits = n), 50, 10000)}
					</SettingRow>
				{/if}
			</SettingsGroup>
		{/if}

		{#if shows('audio', sectionTitles.audio)}
			<SettingsGroup id="audio" title={sectionTitles.audio} description={summaries.audio} bind:open={opened.audio}>
				<SettingRow label={t('settings.transcode.audio.bitrate')}>
					{@render numberInput(t('settings.transcode.audio.bitrate'), settings.audio_bitrate_kbps, (n) => (settings!.audio_bitrate_kbps = n), 32, 640)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.audio.vbr')} hint={t('settings.transcode.audio.vbrHint')}>
					<Switch bare label={t('settings.transcode.audio.vbr')} checked={settings.audio_vbr} onCheckedChange={(checked) => (settings!.audio_vbr = checked)} />
				</SettingRow>
				<SettingRow label={t('settings.transcode.audio.downmix')} hint={t('settings.transcode.audio.downmixHint')}>
					<Switch bare label={t('settings.transcode.audio.downmix')} checked={settings.downmix_audio} onCheckedChange={(checked) => (settings!.downmix_audio = checked)} />
				</SettingRow>
				{#if settings.downmix_audio}
					<SettingRow label={t('settings.transcode.audio.downmixAlgorithm')}>
						{@render select(t('settings.transcode.audio.downmixAlgorithm'), settings.downmix_stereo_algorithm, downmixAlgorithms, (value) => (settings!.downmix_stereo_algorithm = value))}
					</SettingRow>
					<SettingRow label={t('settings.transcode.audio.gain')} hint={t('settings.transcode.audio.gainHint')}>
						{@render numberInput(t('settings.transcode.audio.gain'), settings.downmix_audio_boost, (n) => (settings!.downmix_audio_boost = n), 0.5, 8, 0.5)}
					</SettingRow>
				{/if}
				<SettingRow label={t('settings.transcode.audio.subtitles')} hint={t('settings.transcode.audio.subtitlesHint')}>
					{@render select(t('settings.transcode.audio.subtitles'), settings.subtitle_mode, subtitleModes, (value) => (settings!.subtitle_mode = value as TranscodeSettings['subtitle_mode']))}
				</SettingRow>
				<SettingRow label={t('settings.transcode.audio.extract')} hint={t('settings.transcode.audio.extractHint')}>
					<Switch bare label={t('settings.transcode.audio.extract')} checked={settings.allow_subtitle_extraction !== false} onCheckedChange={(checked) => (settings!.allow_subtitle_extraction = checked)} />
				</SettingRow>
				<SettingRow label={t('settings.transcode.audio.fonts')} hint={t('settings.transcode.audio.fontsHint')}>
					<Switch bare label={t('settings.transcode.audio.fonts')} checked={settings.fallback_font_enabled !== false} onCheckedChange={(checked) => (settings!.fallback_font_enabled = checked)} />
				</SettingRow>
				{#if settings.fallback_font_enabled !== false}
					<SettingRow label={t('settings.transcode.audio.fontDir')}>
						{@render textInput(t('settings.transcode.audio.fontDir'), settings.fallback_font_path, (v) => (settings!.fallback_font_path = v), t('settings.transcode.audio.fontDirPlaceholder'))}
					</SettingRow>
					<SettingRow label={t('settings.transcode.audio.fontName')}>
						{@render textInput(t('settings.transcode.audio.fontName'), settings.fallback_font_name, (v) => (settings!.fallback_font_name = v), t('settings.transcode.audio.fontNamePlaceholder'))}
					</SettingRow>
				{/if}
			</SettingsGroup>
		{/if}

		{#if shows('resources', sectionTitles.resources)}
			<SettingsGroup id="resources" title={sectionTitles.resources} description={summaries.resources} bind:open={opened.resources}>
				<SettingRow label={t('settings.transcode.resources.concurrent')} hint={t('settings.transcode.resources.concurrentHint')}>
					{@render numberInput(t('settings.transcode.resources.concurrent'), settings.max_concurrent, (n) => (settings!.max_concurrent = n), 1, 16)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.queue')} hint={t('settings.transcode.resources.queueHint')}>
					{@render numberInput(t('settings.transcode.resources.queue'), settings.queue_size, (n) => (settings!.queue_size = n), 1, 64)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.threads')} hint={t('settings.transcode.resources.threadsHint')}>
					{@render numberInput(t('settings.transcode.resources.threads'), settings.thread_count, (n) => (settings!.thread_count = n), 0, 64)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.cache')} hint={t('settings.transcode.resources.cacheHint', { size: formatBytes(settings.cache_bytes) || '0 B' })}>
					{@render numberInput(t('settings.transcode.resources.cache'), settings.cache_bytes, (n) => (settings!.cache_bytes = n), undefined, undefined, undefined, true)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.remote')} hint={t('settings.transcode.resources.remoteHint')}>
					{@render numberInput(t('settings.transcode.resources.remote'), settings.remote_bitrate_limit_kbps, (n) => (settings!.remote_bitrate_limit_kbps = n), 0, 1000000)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.mux')} hint={t('settings.transcode.resources.muxHint')}>
					{@render numberInput(t('settings.transcode.resources.mux'), settings.max_muxing_queue_size, (n) => (settings!.max_muxing_queue_size = n), 128, 65536)}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.temp')} hint={t('settings.transcode.resources.tempHint')}>
					{@render textInput(t('settings.transcode.resources.temp'), settings.transcode_temp_path, (v) => (settings!.transcode_temp_path = v), t('settings.transcode.resources.tempPlaceholder'))}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.ffmpeg')}>
					{@render textInput(t('settings.transcode.resources.ffmpeg'), settings.ffmpeg_path, (v) => (settings!.ffmpeg_path = v), t('settings.transcode.resources.pathPlaceholder'))}
				</SettingRow>
				<SettingRow label={t('settings.transcode.resources.ffprobe')}>
					{@render textInput(t('settings.transcode.resources.ffprobe'), settings.ffprobe_path, (v) => (settings!.ffprobe_path = v), t('settings.transcode.resources.pathPlaceholder'))}
				</SettingRow>
			</SettingsGroup>
		{/if}

		{#if shows('capabilities', sectionTitles.capabilities, true)}
			<!-- The probe's raw answer, for diagnosing: closed, since the rows
			     above already say what it means. -->
			<SettingsGroup
				id="capabilities"
				title={sectionTitles.capabilities}
				description={t('settings.transcode.probe.description', { path: capabilities.ffmpeg || 'ffmpeg', count: capabilities.encoders.length })}
				bind:open={opened.capabilities}
			>
				<div class="space-y-4 py-4">
					<div class="flex flex-wrap gap-1.5">
						<Badge tone={capabilities.tone_mapping ? 'success' : 'warning'}>
							{capabilities.tone_mapping ? t('settings.transcode.probe.toneYes') : t('settings.transcode.probe.toneNo')}
						</Badge>
						<Badge tone={capabilities.tone_mapping_bt2390 ? 'success' : 'neutral'}>
							{capabilities.tone_mapping_bt2390 ? t('settings.transcode.probe.bt2390Yes') : t('settings.transcode.probe.bt2390No')}
						</Badge>
						{#each Object.entries(capabilities.hardware) as [backend, ok] (backend)}
							<Badge tone={ok ? 'success' : 'neutral'}>
								{ok ? t('settings.transcode.probe.backendYes', { name: BACKEND_NAMES[backend] ?? backend }) : t('settings.transcode.probe.backendNo', { name: BACKEND_NAMES[backend] ?? backend })}
							</Badge>
						{/each}
					</div>
					<div>
						<p class="font-mono text-[10px] uppercase tracking-[0.16em] text-muted">{t('settings.transcode.probe.encoders')}</p>
						<div class="mt-2 flex flex-wrap gap-1.5">
							{#each capabilities.encoders as encoder (encoder)}<Badge>{encoder}</Badge>{/each}
						</div>
					</div>
				</div>
			</SettingsGroup>
		{/if}

		{#if shows('sessions', sectionTitles.sessions, true)}
			<SettingsGroup
				id="sessions"
				title={sectionTitles.sessions}
				description={t('settings.transcode.sessions.count', { count: activeSessions })}
			>
				{#snippet aside()}
					<Button variant="tonal" size="field" onclick={() => void refreshSessions()}>{t('settings.transcode.sessions.refresh')}</Button>
				{/snippet}
				{#if sessions.length === 0}
					<p class="py-4 text-sm text-muted">{t('settings.transcode.sessions.none')}</p>
				{:else}
					<ul class="settings-items">
						{#each sessions as session (session.session)}
							<li class="flex flex-wrap items-center gap-3 py-2.5 text-sm">
								<Badge tone={session.state === 'failed' ? 'danger' : session.state === 'ready' ? 'success' : 'neutral'}>
									{session.state}
								</Badge>
								<span class="font-mono text-xs text-muted">{session.session.slice(0, 12)}</span>
								<span class="text-xs text-muted">
									{[
										session.delivery ?? 'progressive',
										session.method ?? '—',
										session.encoder,
										session.hardware,
										session.progress ? `${Math.round(session.progress * 100)}%` : '',
										session.fps ? `${session.fps.toFixed(1)} fps` : '',
										session.output_bitrate_kbps ? `${session.output_bitrate_kbps} kbps` : ''
									]
										.filter(Boolean)
										.join(' · ')}
								</span>
								{#if session.fallback}
									<span class="text-xs text-warning">{session.fallback}</span>
								{/if}
								<div class="ms-auto">
									{#if sessionBusy === session.session}
										<Spinner class="size-4 text-muted" />
									{:else}
										<Button variant="ghost" size="field" onclick={() => void cancelSession(session.session)}>
											{t('settings.transcode.sessions.stop')}
										</Button>
									{/if}
								</div>
							</li>
						{/each}
					</ul>
				{/if}
			</SettingsGroup>
		{/if}

		{#if normalizedSearch && !anyShown}
			<div class="rounded-card bg-panel p-10 text-center">
				<Search class="mx-auto size-5 text-muted" />
				<p class="mt-3 text-sm font-medium text-foreground">{t('settings.transcode.noMatch', { query: searchQuery.trim() })}</p>
				<p class="mt-1 text-xs text-muted">{t('settings.transcode.noMatchHint')}</p>
			</div>
		{/if}

		<StagedBar count={changeCount} {saving} onapply={() => void save()} ondiscard={resetChanges} />
	{/if}
</div>
