import { request } from './client';
import type { CatalogPage } from './types';
import type { CatalogSort } from './catalog';

export interface SearchOptions {
	q: string;
	kind?: string;
	limit?: number;
	offset?: number;
	sort?: CatalogSort;
	signal?: AbortSignal;
}

/** Gateway route: GET /api/search (same paged envelope as the catalog). */
export const search = {
	query: (opts: SearchOptions) =>
		request<CatalogPage>('/api/search', {
			query: {
				q: opts.q,
				kind: opts.kind,
				limit: opts.limit,
				offset: opts.offset,
				sort: opts.sort
			},
			signal: opts.signal
		})
};
