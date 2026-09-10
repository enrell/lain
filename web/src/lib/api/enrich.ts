import { ApiError, request } from './client';
import type { Enrichment, EnrichmentBatch } from './types';

/**
 * Gateway routes: POST/GET/DELETE /api/catalog/{id}/enrich and
 * GET /api/enrichments?ids= (batch overlay read for grids).
 *
 * Enrichment is decoration: the catalog owns identity, this overlay
 * only decorates. `get` returns null when the item is not enriched
 * instead of surfacing a 404 as an error.
 */
export const enrich = {
	get: async (id: string): Promise<Enrichment | null> => {
		try {
			return await request<Enrichment>(`/api/catalog/${encodeURIComponent(id)}/enrich`);
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'not-found') return null;
			throw err;
		}
	},

	batch: async (ids: string[]): Promise<Enrichment[]> => {
		if (ids.length === 0) return [];
		const page = await request<EnrichmentBatch>('/api/enrichments', {
			query: { ids: ids.join(',') }
		});
		return page.items ?? [];
	},

	run: (id: string, provider?: string) =>
		request<Enrichment>(`/api/catalog/${encodeURIComponent(id)}/enrich`, {
			method: 'POST',
			query: provider ? { provider } : undefined
		}),

	remove: (id: string) =>
		request<{ status: string }>(`/api/catalog/${encodeURIComponent(id)}/enrich`, {
			method: 'DELETE'
		})
};
