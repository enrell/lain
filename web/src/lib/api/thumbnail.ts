import { mediaUrl } from './client';

export interface ThumbnailOptions {
	/** Seconds into the file; a seek past the end falls back to frame 0 on the server. */
	at?: number;
	/** Output width in pixels (the server clamps to 32..1280). */
	width?: number;
}

/**
 * Gateway route: GET /api/items/{id}/thumbnail.
 *
 * The server extracts the still with ffmpeg and disk-caches it per
 * source mtime/size, timestamp and width, so repeat views are cheap.
 * Image elements cannot attach an Authorization header, so the token
 * rides in the query string exactly like the stream URL — never use
 * mediaUrl for ordinary API calls.
 */
export const thumbnail = {
	url: (id: string, token: string | null, opts: ThumbnailOptions = {}) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/thumbnail`, token, {
			t: String(opts.at ?? 10),
			w: String(opts.width ?? 480)
		})
};
