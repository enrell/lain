import { describe, expect, it } from 'vitest';
import { acquireHref, isReadingMedia, platformName, progressText, statusLabel } from './status';

describe('tracking vocabulary', () => {
	it('says read for what is read', () => {
		expect(statusLabel('current', false)).toBe('Watching');
		expect(statusLabel('current', true)).toBe('Reading');
		expect(statusLabel('repeating', true)).toBe('Rereading');
		expect(statusLabel('paused', true)).toBe('Paused');
		expect(statusLabel('hiatus', false)).toBe('hiatus');
		expect(isReadingMedia('comic')).toBe(true);
		expect(isReadingMedia('anime')).toBe(false);
	});

	it('counts in the entry’s own units', () => {
		expect(progressText({ media_type: 'anime', progress: 12, progress_total: 28 })).toBe('12/28 ep');
		expect(progressText({ media_type: 'anime', progress: 3 })).toBe('3 ep');
		expect(progressText({ media_type: 'manga', progress: 40, progress_volumes: 3 })).toBe('40 ch · vol 3');
	});

	it('names platforms and builds the release search', () => {
		expect(platformName('anilist')).toBe('AniList');
		expect(platformName('mal')).toBe('mal');
		expect(acquireHref({ title: 'Frieren: Beyond', media_type: 'anime' })).toBe('/acquire?q=Frieren%3A+Beyond&kind=anime');
		expect(acquireHref({ title: 'One Piece', media_type: 'manga' })).toBe('/acquire?q=One+Piece&kind=manga');
	});
});
