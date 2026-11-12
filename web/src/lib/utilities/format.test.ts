import { describe, expect, it } from 'vitest';
import type { CatalogItem } from '$lib/api/types';
import { elapsedSince, mediaSubtitle, timeLeft } from './format';

describe('elapsedSince', () => {
	it('counts seconds from the server timestamp', () => {
		expect(elapsedSince(1000, 1_012_500)).toBeCloseTo(12.5);
	});

	it('never goes negative when the server clock leads the browser', () => {
		expect(elapsedSince(2000, 1_000_000)).toBe(0);
	});

	it('returns zero without a timestamp', () => {
		expect(elapsedSince(0, 1_000_000)).toBe(0);
		expect(elapsedSince(undefined, 1_000_000)).toBe(0);
		expect(elapsedSince(null, 1_000_000)).toBe(0);
	});
});

describe('timeLeft', () => {
	it('says what is left the same way everywhere, seconds included', () => {
		expect(timeLeft(7, 16)).toBe('9 sec left');
		expect(timeLeft(0, 24 * 60)).toBe('24 min left');
		// Never "0 sec left" for the last moment.
		expect(timeLeft(16, 16)).toBe('1 sec left');
	});
});

describe('mediaSubtitle', () => {
	const item = (partial: Partial<CatalogItem>): CatalogItem => ({
		id: 'i', library_id: 'l', kind: 'video', title: 'Film', season: 0, episode: 0, year: 0,
		file_path: '', size: 0, confidence: 1, origin: '', provenance: '', updated_at: 0, ...partial
	});
	it('says what exists and never the raw kind', () => {
		expect(mediaSubtitle(item({ year: 2019 }))).toBe('2019');
		expect(mediaSubtitle(item({ kind: 'episode', season: 2, episode: 5 }))).toBe('S02E05');
		expect(mediaSubtitle(item({}))).toBe('');
	});
});
