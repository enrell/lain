import type { TranscodeCapabilities, TranscodeSettings } from '$lib/api/types';

/** Transcode settings are JSON data, so a JSON clone preserves their wire shape. */
export function cloneTranscodeSettings(settings: TranscodeSettings): TranscodeSettings {
	return JSON.parse(JSON.stringify(settings)) as TranscodeSettings;
}

export function settingsEqual(
	left: TranscodeSettings | null,
	right: TranscodeSettings | null
): boolean {
	if (!left || !right) return left === right;
	return JSON.stringify(left) === JSON.stringify(right);
}

/**
 * Stable preference when more than one API passes the server's real encode
 * probe. Keep this mirrored with gateway.automaticHardwarePreference.
 */
const HARDWARE_PREFERENCE = [
	'nvenc',
	'qsv',
	'vaapi',
	'videotoolbox',
	'amf',
	'rkmpp',
	'v4l2m2m'
];

/** Hardware choices come only from the server's real ffmpeg probe. */
export function availableHardwareBackends(capabilities: TranscodeCapabilities | null): string[] {
	if (!capabilities) return [];
	return HARDWARE_PREFERENCE.filter((backend) => capabilities.hardware[backend] === true);
}

/**
 * The choices of Transcoding's video engine control, in order: the backend
 * that is set (kept even when the probe no longer passes it, so the control
 * still shows what is in force), every backend the probe passed, then
 * software ('none') last.
 */
export function engineChoices(current: string, ready: string[]): string[] {
	const hardware = current && current !== 'none' ? [current] : [];
	return [...new Set([...hardware, ...ready]), 'none'];
}

/**
 * Stage a conservative automatic policy using only existing contract values.
 * The best backend that passed the server's real encode probe is enabled;
 * without one, software remains the honest fallback. Capacity, custom paths
 * and the quality ladder remain operator-owned.
 */
export function applySafeAutomaticPolicy(
	current: TranscodeSettings,
	capabilities: TranscodeCapabilities | null
): TranscodeSettings {
	const next = cloneTranscodeSettings(current);
	next.default_delivery = 'hls';
	next.hls_segment_container = 'fmp4';
	next.hls_segment_seconds = 6;
	next.throttle = true;
	next.throttle_ahead_sec = 30;
	next.segment_deletion = false;
	next.idle_timeout_sec = 120;
	next.encoder_preset = 'veryfast';
	next.crf = 23;
	next.h264_preset = '';
	next.h265_preset = '';
	next.av1_preset = '';
	next.allow_hevc = false;
	next.allow_av1 = false;
	next.thread_count = 0;
	next.subtitle_mode = 'auto';
	next.allow_subtitle_extraction = true;
	next.tone_mapping = capabilities?.tone_mapping ?? current.tone_mapping;
	next.tone_mapping_mode = 'auto';
	next.tone_mapping_algorithm = capabilities?.tone_mapping_bt2390 ? 'bt2390' : 'hable';
	next.deinterlace = 'auto';
	next.deinterlace_method = 'yadif';
	next.deinterlace_double_rate = false;
	const hardware = availableHardwareBackends(capabilities)[0] ?? 'none';
	next.hardware_acceleration = hardware;
	next.hardware_encode = hardware !== 'none';
	next.hardware_low_power = false;
	next.hardware_decode_10bit_hevc = false;
	next.hardware_decode_10bit_vp9 = false;
	return next;
}

/**
 * Whether saved settings are what Auto would stage from this probe. A
 * BT.2390 tone map the probe cannot run is the Hable one the server falls
 * back to, so a fresh server's defaults read as Auto, not as a choice
 * nobody made.
 */
export function isAutomatic(settings: TranscodeSettings, capabilities: TranscodeCapabilities | null): boolean {
	const saved = cloneTranscodeSettings(settings);
	if (saved.tone_mapping_algorithm === 'bt2390' && capabilities && !capabilities.tone_mapping_bt2390) {
		saved.tone_mapping_algorithm = 'hable';
	}
	// An older server leaves the switch out; absent means on.
	if (saved.allow_subtitle_extraction === undefined) saved.allow_subtitle_extraction = true;
	return settingsEqual(applySafeAutomaticPolicy(cloneTranscodeSettings(settings), capabilities), saved);
}
