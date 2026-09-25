import { request } from './client';
import type { CatalogItem } from './types';

/**
 * Gateway route: DELETE /api/items/{id} (admin only).
 *
 * Deleting removes the file from disk; the catalog entry is never
 * hard-deleted — it comes back as `missing` (D-068/D-073), so progress
 * and metadata stay attached and the entry restores itself if the path
 * returns on a later scan.
 */
export const items = {
	remove: (id: string) =>
		request<CatalogItem>(`/api/items/${encodeURIComponent(id)}`, { method: 'DELETE' })
};
