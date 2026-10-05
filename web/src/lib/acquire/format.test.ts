import { describe, expect, it } from 'vitest';
import {
	compressUnits,
	formatSeasonMap,
	formatSeasonWants,
	grabProgress,
	isActiveGrab,
	parseCategories,
	parseSeasonMap,
	parseSeasonWants,
	ratio,
	releaseNumbers,
	releaseTags,
	settingsChanges,
	unitLabel
} from './format';
import type { AcquireSettings, Release } from '$lib/api';

const r = (x: Partial<Release>): Release => ({ title: 'Show', parser: 'lain-release-tokenizer', ...x });

describe('acquire format', () => {
	it('names the numbering of each release shape', () => {
		expect(releaseNumbers(r({ season: 1, episodes: [2] }))).toBe('S01E02');
		expect(releaseNumbers(r({ season: 1, episodes: [5, 6] }))).toBe('S01E05–E06');
		expect(releaseNumbers(r({ season: 2, season_pack: true }))).toBe('S02 pack');
		expect(releaseNumbers(r({ episodes: [5], absolute: true }))).toBe('05');
		expect(releaseNumbers(r({ episodes: [1, 2, 3], absolute: true }))).toBe('01–03');
		expect(releaseNumbers(r({ volume: 1, chapter: 3 }))).toBe('v01 c003');
		expect(releaseNumbers(r({}))).toBe('');
	});
	it('lists quality tags', () => {
		expect(releaseTags(r({ resolution: '1080p', source: 'web', codec: 'h265', proper: true, version: 2 }))).toEqual([
			'1080p',
			'web',
			'h265',
			'proper',
			'v2'
		]);
	});
	it('computes progress and ratio', () => {
		expect(grabProgress({ state: 'downloading', completed: 50, size: 200 })).toBe(0.25);
		expect(grabProgress({ state: 'metadata', completed: 0, size: 0 })).toBe(0);
		expect(grabProgress({ state: 'seeding', completed: 0, size: 10 })).toBe(1);
		expect(ratio({ uploaded: 150, size: 100 })).toBe('1.50');
		expect(ratio({ uploaded: 0, size: 0 })).toBe('—');
		expect(isActiveGrab('importing')).toBe(true);
		expect(isActiveGrab('seeding')).toBe(false);
	});
	it('parses category lists', () => {
		expect(parseCategories('5000, 5070 ,x 5070 -1 2.5')).toEqual([5000, 5070]);
		expect(parseCategories('')).toEqual([]);
	});
	it('counts staged setting changes', () => {
		const s: AcquireSettings = {
			dir: '/d', listen_port: 51413, max_active: 3, max_peers: 40, upload_kbps: 0, download_kbps: 0,
			seed_ratio: 1, seed_minutes: 1440, remove_after_seeding: true, import_mode: 'hardlink',
			automation: false, rss_minutes: 30, search_hours: 12, stall_hours: 6, subtitle_hours: 24
		};
		expect(settingsChanges(s, { ...s })).toBe(0);
		expect(settingsChanges(s, { ...s, seed_ratio: 2, import_mode: 'copy' })).toBe(2);
	});
});


describe('monitored title helpers', () => {
	it('parses and prints season maps', () => {
		expect(parseSeasonMap('1:1, 2:13 3:25')).toEqual([
			{ season: 1, first: 1 },
			{ season: 2, first: 13 },
			{ season: 3, first: 25 }
		]);
		expect(parseSeasonMap('')).toEqual([]);
		expect(parseSeasonMap('1:1, x, 2:')).toBeNull();
		expect(formatSeasonMap([{ season: 1, first: 1 }, { season: 2, first: 13 }])).toBe('1:1, 2:13');
	});
	it('parses and prints season ranges', () => {
		expect(parseSeasonWants('1:1-12, 2:1-')).toEqual([
			{ season: 1, from: 1, to: 12 },
			{ season: 2, from: 1, to: 0 }
		]);
		expect(parseSeasonWants('1')).toEqual([{ season: 1, from: 1, to: 0 }]);
		expect(parseSeasonWants('1:12-3')).toBeNull();
		expect(formatSeasonWants([{ season: 1, from: 1, to: 12 }, { season: 2, from: 1, to: 0 }])).toBe('1:1-12, 2:1-');
	});
	it('labels units in the numbering of the title', () => {
		expect(unitLabel({ season: 1, number: 5 }, 'seasonal')).toBe('S01E05');
		expect(unitLabel({ season: 2, number: 0 }, 'seasonal')).toBe('S02');
		expect(unitLabel({ season: 0, number: 7 }, 'absolute')).toBe('07');
		expect(unitLabel({ season: 0, number: 25 }, 'chapter')).toBe('c025');
		expect(unitLabel({ season: 0, number: 3 }, 'volume')).toBe('v03');
	});
	it('compresses runs', () => {
		const u = (n: number, s = 0) => ({ season: s, number: n });
		expect(compressUnits([u(1), u(2), u(3), u(7), u(9), u(10)], 'absolute')).toBe('01–03, 07, 09–10');
		expect(compressUnits([u(1, 1), u(2, 1), u(1, 2)], 'seasonal')).toBe('S01E01–S01E02, S02E01');
		expect(compressUnits([], 'absolute')).toBe('');
	});
});
