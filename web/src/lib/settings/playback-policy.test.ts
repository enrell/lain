import { describe, expect, it } from 'vitest';
import type { TranscodeCapabilities, TranscodeSettings } from '$lib/api/types';
import {
	applySafeAutomaticPolicy,
	availableHardwareBackends,
	cloneTranscodeSettings,
	settingsEqual
} from './playback-policy';

const settings: TranscodeSettings = {
	default_delivery: 'progressive',
	hls_segment_seconds: 10,
	hls_segment_container: 'mpegts',
	throttle: false,
	throttle_ahead_sec: 90,
	segment_deletion: true,
	segment_keep_sec: 120,
	idle_timeout_sec: 300,
	cache_bytes: 10 * 1024 ** 3,
	queue_size: 4,
	max_concurrent: 3,
	thread_count: 8,
	max_muxing_queue_size: 4096,
	transcode_temp_path: '/srv/transcode',
	remote_bitrate_limit_kbps: 12000,
	encoder_preset: 'slow',
	crf: 19,
	h264_preset: 'medium',
	h265_preset: 'slow',
	av1_preset: 'veryslow',
	h264_crf: 20,
	h265_crf: 25,
	av1_crf: 30,
	allow_hevc: true,
	allow_av1: true,
	qualities: [{ name: 'custom', max_width: 1920, max_height: 1080, bitrate_kbps: 5000 }],
	audio_bitrate_kbps: 256,
	audio_vbr: true,
	downmix_audio: false,
	downmix_audio_boost: 1,
	downmix_stereo_algorithm: 'nightmode',
	subtitle_mode: 'burn',
	allow_subtitle_extraction: false,
	fallback_font_path: '/fonts',
	fallback_font_enabled: false,
	fallback_font_name: 'Example Sans',
	tone_mapping: false,
	tone_mapping_algorithm: 'clip',
	tone_mapping_mode: 'never',
	tone_mapping_peak_nits: 400,
	deinterlace: 'off',
	deinterlace_method: 'bwdif',
	deinterlace_double_rate: true,
	hardware_acceleration: 'vaapi',
	hardware_decode_codecs: ['h264'],
	hardware_encode: true,
	hardware_low_power: true,
	hardware_decode_10bit_hevc: true,
	hardware_decode_10bit_vp9: true,
	hardware_device: '/dev/dri/renderD129',
	ffmpeg_path: '/opt/ffmpeg',
	ffprobe_path: '/opt/ffprobe'
};

const capabilities: TranscodeCapabilities = {
	ffmpeg: '/usr/bin/ffmpeg',
	encoders: ['libx264', 'h264_vaapi'],
	tone_mapping: true,
	tone_mapping_bt2390: false,
	hardware: { vaapi: true, nvenc: false, qsv: true }
};

describe('playback settings policy helpers', () => {
	it('lists only hardware backends that passed the server probe', () => {
		expect(availableHardwareBackends(capabilities)).toEqual(['qsv', 'vaapi']);
	});

	it('stages the best probed hardware policy without erasing operator-owned resources', () => {
		const automatic = applySafeAutomaticPolicy(settings, capabilities);

		expect(automatic).not.toBe(settings);
		expect(automatic.default_delivery).toBe('hls');
		expect(automatic.hls_segment_container).toBe('fmp4');
		expect(automatic.thread_count).toBe(0);
		expect(automatic.subtitle_mode).toBe('auto');
		expect(automatic.deinterlace).toBe('auto');
		expect(automatic.tone_mapping).toBe(true);
		expect(automatic.tone_mapping_mode).toBe('auto');
		expect(automatic.tone_mapping_algorithm).toBe('hable');
		expect(automatic.hardware_acceleration).toBe('qsv');
		expect(automatic.hardware_encode).toBe(true);

		// Auto must not silently replace capacity, quality, paths or the ladder.
		expect(automatic.cache_bytes).toBe(settings.cache_bytes);
		expect(automatic.qualities).toEqual(settings.qualities);
		expect(automatic.transcode_temp_path).toBe(settings.transcode_temp_path);
		expect(automatic.ffmpeg_path).toBe(settings.ffmpeg_path);
		expect(settings.hardware_acceleration).toBe('vaapi');
	});

	it('keeps automatic policy on software when no backend passes the real probe', () => {
		const automatic = applySafeAutomaticPolicy(settings, {
			...capabilities,
			hardware: { vaapi: false, qsv: false }
		});
		expect(automatic.hardware_acceleration).toBe('none');
		expect(automatic.hardware_encode).toBe(false);
	});

	it('uses BT.2390 only when the existing capability probe says it is available', () => {
		const automatic = applySafeAutomaticPolicy(settings, {
			...capabilities,
			tone_mapping_bt2390: true
		});
		expect(automatic.tone_mapping_algorithm).toBe('bt2390');
	});

	it('clones nested quality rows and compares settings structurally', () => {
		const clone = cloneTranscodeSettings(settings);
		expect(settingsEqual(settings, clone)).toBe(true);
		clone.qualities[0].name = 'changed';
		expect(settings.qualities[0].name).toBe('custom');
		expect(settingsEqual(settings, clone)).toBe(false);
	});
});
