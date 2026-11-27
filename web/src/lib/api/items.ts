import { request } from './client';
import type { CatalogItem } from './types';

/**
 * Gateway route: DELETE /api/items/{id} (admin only).
 *
 * Deleting removes the file from disk, with its subtitle sidecars, the
 * folders it leaves empty and the torrent copy that still holds it
 * (D-301); the answer's `removal` says what happened. The catalog entry
 * is never hard-deleted: it comes back as `missing` (D-068/D-073), so
 * progress and metadata stay attached and the entry restores itself if
 * the path returns on a later scan. A title with no file left is off the
 * lists.
 */
export const items = {
	remove: (id: string) =>
		request<CatalogItem>(`/api/items/${encodeURIComponent(id)}`, { method: 'DELETE' })
};
