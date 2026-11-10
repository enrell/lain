import { describe, expect, it } from 'vitest';
import type { CatalogItem, Enrichment } from '$lib/api/types';
import { genreCounts, prefersWide, withGenres } from './browse';
import { groupItems } from './grouping';

function item(id: string, title: string, episode = 0): CatalogItem {
	return {
		id, library_id: 'lib', kind: episode ? 'episode' : 'video', title, season: 0, episode, year: 0,
		file_path: `${id}.mkv`, size: 1, confidence: 1, origin: '', provenance: '', updated_at: 1
	};
}

function lookup(map: Record<string, Partial<Enrichment>>) {
	return (id: string) => (map[id] ? ({ item_id: id, provider: 'p', remote_id: '', title: '', fetched_at: 0, ...map[id] } as Enrichment) : null);
}

const groups = groupItems([item('a1', 'Alpha', 1), item('a2', 'Alpha', 2), item('b', 'Beta'), item('c', 'Gamma')]);

describe('prefersWide', () => {
	it('is landscape when most titles have no poster art', () => {
		expect(prefersWide(groups, lookup({ a1: { poster: 'p.jpg' } }), false)).toBe(true);
		expect(prefersWide(groups, lookup({ a1: { poster: 'p.jpg' }, b: { cover: 'c.jpg' } }), false)).toBe(false);
	});
	it('stays portrait for reading libraries and for nothing at all', () => {
		expect(prefersWide(groups, lookup({}), true)).toBe(false);
		expect(prefersWide([], lookup({}), false)).toBe(false);
	});
});

describe('genres', () => {
	const e = lookup({ a1: { genres: ['Drama'] }, a2: { genres: ['drama', 'Mystery'] }, b: { genres: ['Mystery'] } });
	it('counts a genre once per title, whatever the case', () => {
		expect(genreCounts(groups, e)).toEqual([
			{ genre: 'Mystery', count: 2 },
			{ genre: 'Drama', count: 1 }
		]);
	});
	it('keeps the titles that carry every picked genre', () => {
		expect(withGenres(groups, [], e)).toHaveLength(3);
		expect(withGenres(groups, ['mystery'], e).map((g) => g.title)).toEqual(['Alpha', 'Beta']);
		expect(withGenres(groups, ['Mystery', 'Drama'], e).map((g) => g.title)).toEqual(['Alpha']);
	});
});
