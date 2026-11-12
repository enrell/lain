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
/**
 * The most ids one read takes (the server's `MAX_ENRICH_BATCH`): a longer
 * list, a long show's episodes or a Home of many rows, is read in several,
 * where one request was refused whole and every card lost its overlay.
 */
export const ENRICH_BATCH = 200;

export const enrich = {
	batch: async (ids: string[]): Promise<Enrichment[]> => {
		if (ids.length === 0) return [];
		const parts: string[][] = [];
		for (let i = 0; i < ids.length; i += ENRICH_BATCH) parts.push(ids.slice(i, i + ENRICH_BATCH));
		const pages = await Promise.all(
			parts.map((part) => request<EnrichmentBatch>('/api/enrichments', { query: { ids: part.join(',') } }))
		);
		return pages.flatMap((page) => page.items ?? []);
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
