import { describe, expect, it } from 'vitest';
import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
import { NO_FILTERS, activeCount, anyCount, apply, countBy, filtersFromParams, filtersToParams, genreFacets, watchState, withFilters, type Context, type Filters } from './browse';
import { groupItems } from './grouping';

const NOW = 1_000_000;
function item(id: string, title: string, episode = 0, extra: Partial<CatalogItem> = {}): CatalogItem {
	return {
		id, library_id: 'lib', kind: episode ? 'episode' : 'video', title, season: 0, episode, year: 0,
		file_path: `${id}.mkv`, size: 1, confidence: 1, origin: '', provenance: '', updated_at: NOW - 86400 * 40, ...extra
	};
}
const E: Record<string, Partial<Enrichment>> = {
	a1: { genres: ['Drama'] }, a2: { genres: ['drama', 'Mystery'] }, b: { genres: ['Mystery'], title: 'Béta Story' }, c: { genres: ['Travel'] }
};
const P: Record<string, Partial<Progress>> = { a1: { completed: true }, a2: { position_sec: 30 }, b: { completed: true } };
const ctx: Context = {
	enrichment: (id) => (E[id] ? ({ item_id: id, provider: 'p', remote_id: '', title: '', fetched_at: 0, ...E[id] } as Enrichment) : null),
	progress: (id) => (P[id] ? ({ item_id: id, user_id: 'u', position_sec: 0, duration_sec: 100, completed: false, updated_at: 0, ...P[id] } as Progress) : undefined),
	now: NOW
};
const groups = groupItems([
	item('a1', 'Alpha', 1, { year: 2019 }), item('a2', 'Alpha', 2, { year: 2019 }),
	item('b', 'Beta', 0, { year: 2021, size: 900 }), item('c', 'Gamma', 0, { updated_at: NOW - 3600 })
]);
const f = (over: Partial<Filters>): Filters => ({ ...NO_FILTERS, ...over });
const titles = (over: Partial<Filters>) => apply(groups, f(over), ctx).map((g) => g.title);

describe('filters', () => {
	it('searches titles without minding case, accents or word order', () => {
		expect(titles({ q: 'ALPH' })).toEqual(['Alpha']);
		expect(titles({ q: 'story beta' })).toEqual(['Beta']);
		expect(titles({ q: 'nothing' })).toEqual([]);
	});
	it('needs every picked genre and none of the excluded ones', () => {
		expect(titles({ genres: ['mystery'] })).toEqual(['Alpha', 'Beta']);
		expect(titles({ genres: ['Mystery', 'Drama'] })).toEqual(['Alpha']);
		expect(titles({ exclude: ['Drama'] })).toEqual(['Beta', 'Gamma']);
	});
	it('narrows by watching, shape and when it was added', () => {
		expect(groups.map((g) => watchState(g, ctx.progress))).toEqual(['in_progress', 'watched', 'unwatched']);
		expect(titles({ watch: 'unwatched' })).toEqual(['Gamma']);
		expect(titles({ shape: 'series' })).toEqual(['Alpha']);
		expect(titles({ added: 7 })).toEqual(['Gamma']);
	});
	it('sorts', () => {
		expect(titles({ sort: 'title_desc' })).toEqual(['Gamma', 'Beta', 'Alpha']);
		expect(titles({ sort: 'recent' })[0]).toBe('Gamma');
		expect(titles({ sort: 'year_desc' })).toEqual(['Beta', 'Alpha', 'Gamma']);
		expect(titles({ sort: 'year' })).toEqual(['Alpha', 'Beta', 'Gamma']);
		expect(titles({ sort: 'size_desc' })[0]).toBe('Beta');
		expect(titles({ sort: 'episodes_desc' })[0]).toBe('Alpha');
		// The order they came in: what favourites were kept in, newest first.
		expect(apply([...groups].reverse(), f({ sort: 'listed' }), ctx).map((g) => g.title)).toEqual(['Gamma', 'Beta', 'Alpha']);
	});
});

describe('facets', () => {
	it('count what a pick would leave, and keep picked genres listed', () => {
		expect(genreFacets(groups, f({}), ctx)).toEqual([
			{ value: 'Mystery', count: 2 }, { value: 'Drama', count: 1 }, { value: 'Travel', count: 1 }
		]);
		expect(genreFacets(groups, f({ genres: ['Travel'], watch: 'watched' }), ctx)).toEqual([{ value: 'Travel', count: 0 }]);
	});
	it('ignore their own axis', () => {
		const counts = countBy(groups, f({ watch: 'watched' }), ctx, 'watch', (g) => [watchState(g, ctx.progress)]);
		expect(Object.fromEntries(counts)).toEqual({ in_progress: 1, watched: 1, unwatched: 1 });
	});
	it('count an axis left open as everything the other filters keep', () => {
		expect(anyCount(groups, f({ watch: 'watched' }), ctx, 'watch')).toBe(3);
		expect(anyCount(groups, f({ watch: 'watched', genres: ['Mystery'] }), ctx, 'watch')).toBe(2);
	});
});

describe('URL', () => {
	it('round-trips and leaves defaults out', () => {
		const set = f({ q: ' road ', sort: 'recent', genres: ['Sci-Fi', 'Slice of Life'], exclude: ['Horror'], watch: 'in_progress', added: 30, shape: 'series' });
		const params = filtersToParams(set);
		expect(filtersFromParams(params)).toEqual({ ...set, q: 'road' });
		expect(filtersToParams(NO_FILTERS).toString()).toBe('');
		expect(activeCount(set)).toBe(6);
	});
	it('replaces the filters and keeps the page’s own parameters', () => {
		const current = new URLSearchParams('view=all&q=old&genre=Drama');
		expect(withFilters(current, f({ q: 'road', sort: 'recent' })).toString()).toBe('view=all&q=road&sort=recent');
		expect(withFilters(current, NO_FILTERS).toString()).toBe('view=all');
		expect(current.toString()).toBe('view=all&q=old&genre=Drama');
	});
	it('ignores values it does not know', () => {
		expect(filtersFromParams(new URLSearchParams('sort=bogus&watch=x&added=5&shape=y'))).toEqual(NO_FILTERS);
	});
	it('takes the listed order only where it is offered', () => {
		expect(filtersFromParams(new URLSearchParams('sort=listed')).sort).toBe('title');
		const offered = ['listed', 'title'] as const;
		expect(filtersFromParams(new URLSearchParams(''), 'listed', offered).sort).toBe('listed');
		expect(filtersFromParams(new URLSearchParams('sort=title'), 'listed', offered).sort).toBe('title');
		expect(filtersToParams(f({ sort: 'listed' }), 'listed').toString()).toBe('');
	});
});
