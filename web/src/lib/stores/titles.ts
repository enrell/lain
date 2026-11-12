import { api } from '$lib/api';
import { groupItems, type SeriesGroup } from '$lib/utilities/grouping';

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
