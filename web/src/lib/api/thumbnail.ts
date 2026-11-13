import { mediaUrl, request } from './client';

export interface ThumbnailOptions {
	/**
	 * Seconds into the file; a seek past the end falls back to frame 0 on the
	 * server. Left out, the server picks the cover: the still that came with
	 * the item when it has one, else the frame at 10 s.
	 */
	at?: number;
	/** Output width in pixels (the server clamps to 32..1280). */
	width?: number;
	/**
	 * `still`: the file's own picture (D-284), its `-thumb` still or its
	 * frame, never the show's poster kept beside a first episode. Left out,
	 * the cover that stands for the title. An older server ignores it.
	 */
	art?: 'still';
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
			...(opts.at === undefined ? {} : { t: String(opts.at) }),
			...(opts.art ? { art: opts.art } : {}),
			w: String(opts.width ?? 480)
		})
};

/** How an item's storyboard is laid out: tile `i`, row by row, is the frame near `i * interval_sec`. */
export interface StoryboardLayout {
	interval_sec: number;
	count: number;
	columns: number;
	rows: number;
	tile_width: number;
	tile_height: number;
}

/**
 * Gateway routes: GET /api/items/{id}/storyboard (the layout, cheap) and
 * /storyboard/image (one JPEG of small frames; the first request for an
 * item waits while the server makes it). A seek bar cuts its previews out
 * of the image, so dragging fetches nothing.
 */
export const storyboard = {
	layout: (id: string) => request<StoryboardLayout>(`/api/items/${encodeURIComponent(id)}/storyboard`),
	imageUrl: (id: string, token: string | null) => mediaUrl(`/api/items/${encodeURIComponent(id)}/storyboard/image`, token)
};
