import { request } from './client';
import type { CatalogItem, CatalogPage } from './types';

export type CatalogSort = 'title' | 'recent';

export interface CatalogListOptions {
	limit?: number;
	offset?: number;
	sort?: CatalogSort;
	/** Restrict to one library (server-side filter). */
	libraryId?: string;
	signal?: AbortSignal;
}

/** Gateway routes: GET /api/catalog, GET /api/catalog/{id}. */
export const catalog = {
	list: (opts: CatalogListOptions = {}) =>
		request<CatalogPage>('/api/catalog', {
			query: {
				limit: opts.limit,
				offset: opts.offset,
				sort: opts.sort,
				library_id: opts.libraryId
			},
			signal: opts.signal
		}),

	get: (id: string) => request<CatalogItem>(`/api/catalog/${encodeURIComponent(id)}`)
};
