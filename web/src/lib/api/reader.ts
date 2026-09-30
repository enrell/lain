import { mediaUrl, request } from './client';
import type { ReaderView } from './types';

/**
 * Gateway routes: GET /api/items/{id}/pages and /pages/{n}. Page images
 * are <img> sources, which cannot carry an Authorization header, so the
 * token rides in the query string exactly like the stream URL.
 */
export const reader = {
	pages: (id: string) => request<ReaderView>(`/api/items/${encodeURIComponent(id)}/pages`),

	pageUrl: (id: string, index: number, token: string | null) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/pages/${index}`, token)
};
