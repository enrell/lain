import { mediaUrl, request } from './client';
import type { BookManifest, ReaderView } from './types';

/**
 * Gateway routes: GET /api/items/{id}/pages and /pages/{n}, and the book
 * routes (D-306). Page images and a book's resources are <img> and CSS
 * sources, which cannot carry an Authorization header, so the token
 * rides in the query string exactly like the stream URL.
 */
export const reader = {
	pages: (id: string) => request<ReaderView>(`/api/items/${encodeURIComponent(id)}/pages`),

	/** A page image; a PDF page is rendered `width` pixels wide. */
	pageUrl: (id: string, index: number, token: string | null, width?: number) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/pages/${index}`, token, width ? { w: String(width) } : {}),

	book: (id: string) => request<BookManifest>(`/api/items/${encodeURIComponent(id)}/book`),

	/** A chapter as sanitized HTML (its resources still need the token). */
	chapter: (id: string, index: number) =>
		request<string>(`/api/items/${encodeURIComponent(id)}/book/chapters/${index}`)
};
