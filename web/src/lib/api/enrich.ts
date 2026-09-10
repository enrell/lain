import { request } from './client';
import type { Enrichment, EnrichmentBatch } from './types';

/**
 * Gateway routes: POST/DELETE /api/catalog/{id}/enrich and
 * GET /api/enrichments?ids= (batch overlay read for grids and detail).
 *
 * Reads go through the batch endpoint on purpose: "no overlay yet" is
 * a normal state, and the per-item GET would render it as a 404 in the
 * browser console for every un-enriched item.
 */
export const enrich = {
	batch: async (ids: string[]): Promise<Enrichment[]> => {
		if (ids.length === 0) return [];
		const page = await request<EnrichmentBatch>('/api/enrichments', {
			query: { ids: ids.join(',') }
		});
		return page.items ?? [];
	},

	/** One item's overlay, or null when it has none. */
	get: async (id: string): Promise<Enrichment | null> => {
		const [first] = await enrich.batch([id]);
		return first ?? null;
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
