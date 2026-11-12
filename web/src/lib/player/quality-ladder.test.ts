import { describe, expect, it } from 'vitest';
import type { TranscodeQuality } from '$lib/api/types';
import { offeredQualities } from './quality-ladder';

const q = (name: string, max_width: number, max_height: number, bitrate_kbps = 0): TranscodeQuality => ({
	name,
	max_width,
	max_height,
	bitrate_kbps
});

// The server's shipped ladder (lain-watch settings::defaults).
const SHIPPED = [
	q('2160p', 3840, 2160, 40000),
	q('1080p', 1920, 1080, 8000),
	q('720p', 1280, 720, 4000),
	q('480p', 854, 480, 1500),
	q('360p', 640, 360, 700)
];

const names = (width: number | undefined, height: number | undefined, ladder = SHIPPED) =>
	offeredQualities(ladder, width, height).map((r) => r.name);

describe('the quality ladder a source is offered', () => {
	it('stops at the rung of a 16:9 file’s own size', () => {
		expect(names(1920, 1080)).toEqual(['1080p', '720p', '480p', '360p']);
		expect(names(1280, 720)).toEqual(['720p', '480p', '360p']);
		expect(names(640, 360)).toEqual(['360p']);
	});

	it('keeps a widescreen film’s own rung, whose height alone is under it', () => {
		// A 2.40:1 1080p film and its 4K release.
		expect(names(1920, 800)).toEqual(['1080p', '720p', '480p', '360p']);
		expect(names(3840, 1600)).toEqual(['2160p', '1080p', '720p', '480p', '360p']);
		// Cropped a few pixels on each side, and 4:3 in an HD frame.
		expect(names(1916, 1036)).toEqual(['1080p', '720p', '480p', '360p']);
		expect(names(1440, 1080)).toEqual(['1080p', '720p', '480p', '360p']);
	});

	it('offers a file smaller than every rung the smallest one', () => {
		expect(names(320, 180)).toEqual(['360p']);
	});

	it('offers the whole ladder to a source larger than its top', () => {
		expect(names(7680, 4320)).toEqual(SHIPPED.map((r) => r.name));
	});

	it('offers the whole ladder when the source size is unknown', () => {
		expect(names(undefined, undefined)).toEqual(SHIPPED.map((r) => r.name));
		expect(names(1920, 0)).toEqual(SHIPPED.map((r) => r.name));
	});

	it('keeps a bitrate-only rung and rungs that share a box', () => {
		const ladder = [q('1080p', 1920, 1080, 8000), q('1080p lite', 1920, 1080, 3000), q('Data saver', 0, 0, 500), q('720p', 1280, 720)];
		expect(names(1920, 1080, ladder)).toEqual(['1080p', '1080p lite', 'Data saver', '720p']);
		expect(names(1280, 536, ladder)).toEqual(['Data saver', '720p']);
	});

	it('reads an unset side as unbounded', () => {
		const ladder = [q('Tall', 0, 2160), q('HD', 0, 1080), q('SD', 0, 480)];
		expect(names(3840, 1600, ladder)).toEqual(['Tall', 'HD', 'SD']);
		expect(names(1920, 800, ladder)).toEqual(['HD', 'SD']);
	});
});
