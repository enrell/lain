import { api } from '$lib/api';
import type { CatalogListOptions } from '$lib/api/catalog';
import type { CatalogItem, CatalogPage } from '$lib/api/types';
import { groupItems, normalizeSeriesTitle, sortGroups, type SeriesGroup } from '$lib/utilities/grouping';

/**
 * A title with every one of its files. A page of the newest files names
 * the titles but holds only a show's latest episodes: its card counted 16
 * of 26 and showed an episode's still where the library page shows the
 * show's poster (its first episode's). A failed lookup keeps the part.
 */
export async function wholeTitle(group: SeriesGroup): Promise<SeriesGroup> {
	try {
		const { items } = await api.catalog.episodes(group.items[0].id);
		return groupItems(items)[0] ?? group;
	} catch {
		return group;
	}
}

/** Whether the server paged titles (D-283): every title on the page is whole and `total` counts titles. */
export function pagedTitles(page: CatalogPage): boolean {
	return page.group === 'title';
}

/** A page of titles, in the server's order, as many as were asked for. */
export interface TitlesPage {
	groups: SeriesGroup[];
	/** How many titles there are in all, or -1 when only files were counted (an older server). */
	total: number;
}

/**
 * The titles of a library (or of every library), whole: one request to a
 * server that pages titles (D-283). An older server pages files, so the
 * files are grouped here and, when the page was cut, each title is asked
 * for again so its card counts all of its episodes (see `wholeTitle`).
 */
export async function listTitles(opts: Omit<CatalogListOptions, 'group' | 'offset'> & { limit: number }): Promise<TitlesPage> {
	let page = await api.catalog.list({ ...opts, group: 'title' });
	if (pagedTitles(page)) return { groups: groupItems(page.items), total: page.total };
	// That page was `limit` files: enough files to name `limit` titles.
	if (page.total > page.items.length) page = await api.catalog.list({ ...opts, limit: Math.min(500, opts.limit * 10) });
	const all = groupItems(page.items);
	const listed = (opts.sort === 'recent' ? sortGroups(all, 'recent') : all).slice(0, opts.limit);
	const cut = page.total > page.items.length;
	const groups = cut ? await Promise.all(listed.map((g) => (g.items.some((i) => i.episode > 0) ? wholeTitle(g) : g))) : listed;
	return { groups, total: -1 };
}

/**
 * The titles some files belong to, whole and in the files' order, each
 * once, the first `limit` of them: a favourite kept on an episode is its
 * show, two episodes of one show kept apart are one title. A part of a
 * series is asked for its title's files; a lookup that fails keeps the
 * file alone.
 */
export async function titlesOf(items: CatalogItem[], limit = Infinity): Promise<SeriesGroup[]> {
	const seen = new Set<string>();
	const firsts = items
		.filter((item) => {
			const key = normalizeSeriesTitle(item.title);
			if (seen.has(key)) return false;
			seen.add(key);
			return true;
		})
		.slice(0, limit);
	return Promise.all(
		firsts.map((item) =>
			item.episode > 0 || item.season > 0 ? wholeTitle(groupItems([item])[0]) : Promise.resolve(groupItems([item])[0])
		)
	);
}
