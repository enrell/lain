import { describe, expect, it } from 'vitest';
import type { CatalogItem } from '$lib/api/types';
import { compareEpisodes, groupItems, normalizeSeriesTitle, sortGroups } from './grouping';

function item(partial: Partial<CatalogItem> & { id: string }): CatalogItem {
	return {
		library_id: 'lib',
		kind: 'episode',
		title: 'Show',
		season: 0,
		episode: 0,
		year: 0,
		file_path: `/x/${partial.id}.mkv`,
		size: 10,
		confidence: 0.9,
		origin: 'p',
		provenance: 'identify:p',
		updated_at: 0,
		...partial
	};
}

describe('normalizeSeriesTitle', () => {
	it('collapses case and whitespace', () => {
		expect(normalizeSeriesTitle('  Darling  in the FranXX ')).toBe('darling in the franxx');
		expect(normalizeSeriesTitle('Silo')).toBe('silo');
	});
});

describe('compareEpisodes', () => {
	it('orders by season then episode', () => {
		const e20 = item({ id: 'a', episode: 20, season: 1 });
		const e9 = item({ id: 'b', episode: 9, season: 1 });
		const s2e1 = item({ id: 'c', episode: 1, season: 2 });
		expect([e20, s2e1, e9].sort(compareEpisodes).map((i) => i.id)).toEqual(['b', 'a', 'c']);
	});
});

describe('groupItems', () => {
	it('collapses one show into a group with episodes in order', () => {
		const groups = groupItems([
			item({ id: 'e20', title: 'Darling in the FranXX', episode: 20 }),
			item({ id: 'e9', title: 'darling in the franxx', episode: 9 }),
			item({ id: 'm1', title: 'Silo', season: 1, episode: 8 })
		]);
		expect(groups).toHaveLength(2);
		const show = groups.find((g) => g.key === 'darling in the franxx');
		expect(show?.count).toBe(2);
		expect(show?.items.map((i) => i.id)).toEqual(['e9', 'e20']);
	});

	it('keeps movies as single-item groups', () => {
		const groups = groupItems([item({ id: 'm', title: 'Some Movie', kind: 'movie', year: 2019 })]);
		expect(groups).toHaveLength(1);
		expect(groups[0].count).toBe(1);
		expect(groups[0].year).toBe(2019);
	});

	it('drops the year when episodes disagree', () => {
		const groups = groupItems([
			item({ id: 'a', title: 'Show', year: 2020 }),
			item({ id: 'b', title: 'Show', year: 0 })
		]);
		expect(groups[0].year).toBe(0);
	});
});

describe('sortGroups', () => {
	it('sorts sections by title or recency', () => {
		const groups = groupItems([
			item({ id: 'b', title: 'Bravo', updated_at: 5 }),
			item({ id: 'a', title: 'Alpha', updated_at: 1 })
		]);
		expect(sortGroups(groups, 'title').map((g) => g.title)).toEqual(['Alpha', 'Bravo']);
		expect(sortGroups(groups, 'recent').map((g) => g.title)).toEqual(['Bravo', 'Alpha']);
	});
});
