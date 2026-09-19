import type { CatalogItem } from '$lib/api/types';

/** One show (or movie) on the Library page: every indexed file that
 * shares a normalized title, with its episodes in watch order. */
export interface SeriesGroup {
	key: string;
	title: string;
	libraryIds: string[];
	year: number;
	count: number;
	latest: number;
	items: CatalogItem[];
}

export function normalizeSeriesTitle(title: string): string {
	return title.toLowerCase().split(/\s+/).filter(Boolean).join(' ');
}

/** Watch order inside a show: season, then episode, then year, then id. */
export function compareEpisodes(a: CatalogItem, b: CatalogItem): number {
	if (a.season !== b.season) return a.season - b.season;
	if (a.episode !== b.episode) return a.episode - b.episode;
	if (a.year !== b.year) return a.year - b.year;
	return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
}

/**
 * The label a title page and the player sidebar show for one file:
 * S01E02 when the file declares a season, Episode 2 when it only
 * declares an episode, and the title itself otherwise (a movie).
 */
export function episodeLabel(item: { season: number; episode: number; title: string }): string {
	if (item.season > 0 && item.episode > 0)
		return `S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`;
	if (item.episode > 0) return `Episode ${item.episode}`;
	return item.title;
}

/**
 * Collapse a flat catalog page into one group per show. Movies (one
 * file, one title) become single-item groups so the grid below can
 * render everything through the same shape.
 */
export function groupItems(items: CatalogItem[]): SeriesGroup[] {
	const byKey = new Map<string, SeriesGroup>();
	for (const item of items) {
		const key = normalizeSeriesTitle(item.title);
		const existing = byKey.get(key);
		if (!existing) {
			byKey.set(key, {
				key,
				title: item.title,
				libraryIds: [item.library_id],
				year: item.year,
				count: 1,
				latest: item.updated_at,
				items: [item]
			});
			continue;
		}
		existing.items.push(item);
		existing.count += 1;
		if (!existing.libraryIds.includes(item.library_id)) {
			existing.libraryIds.push(item.library_id);
		}
		if (existing.year !== item.year) {
			// Episodes of one show often disagree (0 vs the real
			// year): keep a year only when every file agrees.
			existing.year = 0;
		}
		if (item.updated_at > existing.latest) existing.latest = item.updated_at;
		// Keep the first-seen display title; files of one show share
		// it, and the key is already normalized.
	}
	const groups = [...byKey.values()];
	for (const group of groups) {
		group.items.sort(compareEpisodes);
	}
	return groups;
}

export type GroupSort = 'title' | 'recent';

/**
 * Wrap a server-provided episode list in the SeriesGroup shape the title
 * page renders. The catalog owns the grouping rule (its TitleKey matches
 * normalizeSeriesTitle); this only derives the presentation fields, and
 * through the same code the Library grid uses, so a show's card and its
 * page cannot disagree about the episode count.
 */
export function groupFromEpisodes(episodes: CatalogItem[]): SeriesGroup | null {
	return groupItems(episodes)[0] ?? null;
}

/** Order the show sections to match the Library sort control. */
export function sortGroups(groups: SeriesGroup[], sort: GroupSort): SeriesGroup[] {
	const out = [...groups];
	if (sort === 'recent') {
		out.sort((a, b) => b.latest - a.latest || (a.title < b.title ? -1 : 1));
		return out;
	}
	out.sort((a, b) => (a.title < b.title ? -1 : a.title > b.title ? 1 : 0));
	return out;
}
