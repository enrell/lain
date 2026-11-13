import type { Enrichment, Progress } from '$lib/api/types';
import type { SeriesGroup } from './grouping';

type Lookup = (id: string) => Enrichment | null | undefined;
type ProgressLookup = (id: string) => Progress | undefined;

/** The file whose picture stands for a group on its card. */
export function artItem(group: SeriesGroup) {
	return group.items.find((i) => i.episode > 0) ?? group.items[0];
}

/**
 * The file whose overlay a title's poster card draws: the first file of the
 * title with a poster, then with a wide cover, else `artItem`. Overlays are
 * kept per file, and the one a title has may sit on the episode being
 * watched (a backfill that stopped part way, metadata fetched from that
 * episode's page) while the first has none: the card then drew the first
 * episode's frame letterboxed in the poster frame, where the hero drew the
 * title's poster. Progress never chooses the picture.
 */
export function posterItem(group: SeriesGroup, enrichment: Lookup) {
	return (
		group.items.find((i) => !!enrichment(i.id)?.poster) ??
		group.items.find((i) => !!enrichment(i.id)?.cover) ??
		artItem(group)
	);
}

/** Every genre of a group's files, first spelling kept, case ignored. */
export function groupGenres(group: SeriesGroup, enrichment: Lookup): string[] {
	const seen = new Map<string, string>();
	for (const item of group.items) {
		for (const genre of enrichment(item.id)?.genres ?? []) {
			const key = genre.trim().toLowerCase();
			if (key && !seen.has(key)) seen.set(key, genre.trim());
		}
	}
	return [...seen.values()];
}

export const SORTS = [
	{ value: 'title', label: 'Title A–Z' },
	{ value: 'title_desc', label: 'Title Z–A' },
	{ value: 'recent', label: 'Recently added' },
	{ value: 'oldest', label: 'Oldest added' },
	{ value: 'year_desc', label: 'Newest release' },
	{ value: 'year', label: 'Oldest release' },
	{ value: 'size_desc', label: 'Largest' },
	{ value: 'episodes_desc', label: 'Most episodes' }
] as const;
/**
 * The order the titles came in, offered only where that order means
 * something of its own (favourites: the one kept last first). Its label is
 * the caller's, since it says what the order is.
 */
export const LISTED = 'listed';
export type Sort = (typeof SORTS)[number]['value'] | typeof LISTED;
const SORT_VALUES: readonly Sort[] = SORTS.map((s) => s.value);

export const WATCH = [
	{ value: 'unwatched', label: 'Not started' },
	{ value: 'in_progress', label: 'In progress' },
	{ value: 'watched', label: 'Finished' }
] as const;
export type Watch = (typeof WATCH)[number]['value'];

export const ADDED = [
	{ value: 1, label: 'Today' },
	{ value: 7, label: 'This week' },
	{ value: 30, label: 'This month' },
	{ value: 90, label: 'Last 3 months' }
] as const;

export const SHAPES = [
	{ value: 'series', label: 'Series' },
	{ value: 'single', label: 'Single titles' }
] as const;
export type Shape = (typeof SHAPES)[number]['value'];

/** Everything the browse page narrows by. It lives in the URL. */
export interface Filters {
	q: string;
	sort: Sort;
	genres: string[];
	exclude: string[];
	watch: Watch | '';
	added: number;
	shape: Shape | '';
	kind: string;
}

export const NO_FILTERS: Filters = { q: '', sort: 'title', genres: [], exclude: [], watch: '', added: 0, shape: '', kind: '' };

const list = (v: string | null): string[] => (v ? v.split('|').map((s) => s.trim()).filter(Boolean) : []);

/** `sorts`: the orders this page offers; another in the URL falls back to the default. */
export function filtersFromParams(p: URLSearchParams, defaultSort: Sort = 'title', sorts: readonly Sort[] = SORT_VALUES): Filters {
	const sort = p.get('sort');
	const watch = p.get('watch');
	const shape = p.get('shape');
	const added = Number(p.get('added'));
	return {
		q: p.get('q') ?? '',
		sort: sorts.includes(sort as Sort) ? (sort as Sort) : defaultSort,
		genres: list(p.get('genre')),
		exclude: list(p.get('not')),
		watch: WATCH.some((w) => w.value === watch) ? (watch as Watch) : '',
		added: ADDED.some((a) => a.value === added) ? added : 0,
		shape: SHAPES.some((s) => s.value === shape) ? (shape as Shape) : '',
		kind: p.get('kind') ?? ''
	};
}

/** Only what differs from the defaults, so a bare URL is the bare page. */
export function filtersToParams(f: Filters, defaultSort: Sort = 'title'): URLSearchParams {
	const p = new URLSearchParams();
	if (f.q.trim()) p.set('q', f.q.trim());
	if (f.sort !== defaultSort) p.set('sort', f.sort);
	if (f.genres.length) p.set('genre', f.genres.join('|'));
	if (f.exclude.length) p.set('not', f.exclude.join('|'));
	if (f.watch) p.set('watch', f.watch);
	if (f.added) p.set('added', String(f.added));
	if (f.shape) p.set('shape', f.shape);
	if (f.kind) p.set('kind', f.kind);
	return p;
}

/** The parameters `filtersToParams` writes. */
const FILTER_PARAMS = ['q', 'sort', 'genre', 'not', 'watch', 'added', 'shape', 'kind'];

/**
 * The address's parameters with the filters replaced: the page's own
 * (the Library page's `view`) are kept, so narrowing the grid does not
 * drop which view the address names.
 */
export function withFilters(current: URLSearchParams, f: Filters, defaultSort: Sort = 'title'): URLSearchParams {
	const p = new URLSearchParams(current);
	for (const key of FILTER_PARAMS) p.delete(key);
	for (const [key, value] of filtersToParams(f, defaultSort)) p.set(key, value);
	return p;
}

/** How many filters are set (the search text and the sort are not filters). */
export function activeCount(f: Filters): number {
	return f.genres.length + f.exclude.length + (f.watch ? 1 : 0) + (f.added ? 1 : 0) + (f.shape ? 1 : 0) + (f.kind ? 1 : 0);
}

/** Lower case, accents folded, so "pokemon" finds "Pokémon". */
export function fold(text: string): string {
	return text.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();
}

export function watchState(group: SeriesGroup, progress: ProgressLookup): Watch {
	let done = 0;
	let started = 0;
	for (const item of group.items) {
		const p = progress(item.id);
		if (!p) continue;
		if (p.completed) done += 1;
		else if (p.position_sec > 0) started += 1;
	}
	if (done === group.items.length) return 'watched';
	return done + started > 0 ? 'in_progress' : 'unwatched';
}

export interface Context {
	enrichment: Lookup;
	progress: ProgressLookup;
	/** Unix seconds; passed in so the result is a function of its inputs. */
	now: number;
}

type Skip = 'genres' | 'watch' | 'kind' | 'shape' | 'added' | null;

/** Whether a group passes the filters. `skip` leaves one axis out, which
 * is how a facet counts what picking each of its options would give. */
export function matches(group: SeriesGroup, f: Filters, ctx: Context, skip: Skip = null): boolean {
	if (f.q.trim()) {
		const hay = fold([group.title, ...group.items.map((i) => ctx.enrichment(i.id)?.title ?? '')].join('\n'));
		for (const word of fold(f.q).split(/\s+/).filter(Boolean)) if (!hay.includes(word)) return false;
	}
	if (skip !== 'shape' && f.shape && (f.shape === 'series') !== group.count > 1) return false;
	if (skip !== 'kind' && f.kind && !group.items.some((i) => i.kind === f.kind)) return false;
	if (skip !== 'added' && f.added && group.latest < ctx.now - f.added * 86400) return false;
	if (skip !== 'watch' && f.watch && watchState(group, ctx.progress) !== f.watch) return false;
	if (f.genres.length || f.exclude.length) {
		const have = new Set(groupGenres(group, ctx.enrichment).map((g) => g.toLowerCase()));
		if (f.exclude.some((g) => have.has(g.toLowerCase()))) return false;
		if (skip !== 'genres' && !f.genres.every((g) => have.has(g.toLowerCase()))) return false;
	}
	return true;
}

const year = (g: SeriesGroup): number => g.items.reduce((y, i) => Math.max(y, i.year), 0);
const size = (g: SeriesGroup): number => g.items.reduce((n, i) => n + i.size, 0);
const byTitle = (a: SeriesGroup, b: SeriesGroup): number => a.title.localeCompare(b.title, undefined, { numeric: true, sensitivity: 'base' });

export function sortBy(groups: SeriesGroup[], sort: Sort): SeriesGroup[] {
	const out = [...groups];
	const then = (primary: (a: SeriesGroup, b: SeriesGroup) => number) => out.sort((a, b) => primary(a, b) || byTitle(a, b));
	switch (sort) {
		case 'title_desc': return out.sort((a, b) => byTitle(b, a));
		case 'recent': return then((a, b) => b.latest - a.latest);
		case 'oldest': return then((a, b) => a.latest - b.latest);
		// A title with no year sorts last either way.
		case 'year_desc': return then((a, b) => year(b) - year(a));
		case 'year': return then((a, b) => (year(a) || 9999) - (year(b) || 9999));
		case 'size_desc': return then((a, b) => size(b) - size(a));
		case 'episodes_desc': return then((a, b) => b.count - a.count);
		case LISTED: return out;
		default: return out.sort(byTitle);
	}
}

export function apply(groups: SeriesGroup[], f: Filters, ctx: Context): SeriesGroup[] {
	return sortBy(groups.filter((g) => matches(g, f, ctx)), f.sort);
}

export interface Facet {
	value: string;
	count: number;
}

/**
 * Genres with the number of titles each would leave if picked on top of
 * the other filters, most common first. Picked and excluded genres stay
 * listed even at zero, so they can always be turned off.
 */
export function genreFacets(groups: SeriesGroup[], f: Filters, ctx: Context): Facet[] {
	const counts = new Map<string, Facet>();
	for (const g of [...f.genres, ...f.exclude]) counts.set(g.toLowerCase(), { value: g, count: 0 });
	for (const group of groups) {
		if (!matches(group, f, ctx)) continue;
		for (const genre of groupGenres(group, ctx.enrichment)) {
			const entry = counts.get(genre.toLowerCase());
			if (entry) entry.count += 1;
			else counts.set(genre.toLowerCase(), { value: genre, count: 1 });
		}
	}
	return [...counts.values()].sort((a, b) => b.count - a.count || a.value.localeCompare(b.value));
}

/**
 * How many titles a single-choice axis leaves when nothing is picked on
 * it: the count of its "Any" row, which is the way back to everything
 * that does not also clear the other filters.
 */
export function anyCount(groups: SeriesGroup[], f: Filters, ctx: Context, axis: Exclude<Skip, 'genres' | null>): number {
	return groups.filter((group) => matches(group, f, ctx, axis)).length;
}

/** Counts per option of one single-choice axis, ignoring that axis's own pick. */
export function countBy(
	groups: SeriesGroup[],
	f: Filters,
	ctx: Context,
	axis: Exclude<Skip, 'genres' | null>,
	values: (group: SeriesGroup) => string[]
): Map<string, number> {
	const counts = new Map<string, number>();
	for (const group of groups) {
		if (!matches(group, f, ctx, axis)) continue;
		for (const v of values(group)) counts.set(v, (counts.get(v) ?? 0) + 1);
	}
	return counts;
}
