import { request } from './client';
import type { CatalogItem, CatalogPage } from './types';

export type CatalogSort = 'title' | 'recent';

export interface CatalogListOptions {
	limit?: number;
	offset?: number;
	sort?: CatalogSort;
	/** Restrict to one library (server-side filter). */
	libraryId?: string;
	/**
	 * `title` pages titles, not files (D-283): every file of each title on
	 * the page, `total` counting titles. The answer's `group` says whether
	 * the server did; an older one pages files.
	 */
	group?: 'title';
	signal?: AbortSignal;
}

/** Envelope for one title's files (GET /api/catalog/{id}/episodes). */
export interface CatalogItems {
	items: CatalogItem[];
}

/** Gateway routes: GET /api/catalog, GET /api/catalog/{id}, GET /api/catalog/{id}/episodes. */
export const catalog = {
	list: (opts: CatalogListOptions = {}) =>
		request<CatalogPage>('/api/catalog', {
			query: {
				limit: opts.limit,
				offset: opts.offset,
				sort: opts.sort,
				library_id: opts.libraryId,
				group: opts.group
			},
			signal: opts.signal
		}),

	get: (id: string) => request<CatalogItem>(`/api/catalog/${encodeURIComponent(id)}`),

	/** Every file of the item's title, in watch order. The server owns the
	 * grouping rule, so the title page never depends on the search plugin. */
	episodes: (id: string) =>
		request<CatalogItems>(`/api/catalog/${encodeURIComponent(id)}/episodes`)
};
