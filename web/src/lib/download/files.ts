import { mediaUrl } from '$lib/api/client';

/**
 * The browser "save the original file" URL (GET /api/items/{id}/stream
 * ?download=1): the server answers with Content-Disposition: attachment
 * and the original file name. Like every media URL, the token rides in
 * the query string because a plain navigation cannot send a header.
 */
export function downloadUrl(itemId: string, token: string | null): string {
	return mediaUrl(`/api/items/${encodeURIComponent(itemId)}/stream`, token, { download: '1' });
}

/** Items that can be saved: present on disk, de-duplicated, in order. */
export function downloadable<T extends { id: string; missing?: boolean }>(items: T[]): T[] {
	const seen = new Set<string>();
	return items.filter((i) => {
		if (i.missing || seen.has(i.id)) return false;
		seen.add(i.id);
		return true;
	});
}
