import { mediaUrl, request } from './client';
import { reportCapabilities } from '$lib/utilities/capabilities';
import type {
	PlaybackOptions,
	PlaybackPlan,
	TranscodeSelection,
	TranscodeStatus
} from './types';

/**
 * Gateway routes: GET /api/items/{id}/playback (plan),
 * GET /api/items/{id}/stream (bytes, Range-capable),
 * GET /api/items/{id}/transcode (progressive output) and
 * GET /api/items/{id}/transcode/hls/{file} (HLS playlist/segments).
 *
 * The plan is the playback policy: the UI must honor `available` and
 * `mode` instead of reconstructing URLs by itself. A `transcode` plan
 * starts a session (POST) and then plays either the progressive MP4 or
 * the HLS playlist the session reports. Media URLs are the only place
 * a token rides in the query string — native media elements and
 * hls.js cannot send Authorization headers.
 */
export const playback = {
	/**
	 * The plan is the playback policy: the UI must honor `available` and
	 * `mode` instead of reconstructing URLs by itself. The request carries
	 * what this browser reported it can decode (D-058) — a token list the
	 * server validates against its own vocabulary — so a browser that
	 * proves it can decode a Matroska file is not handed a transcode it
	 * does not need. A client that reports nothing keeps the conservative
	 * rules: an absent or empty `caps` is never a licence to direct-play.
	 */
	plan: async (id: string, opts: { client?: string; network?: string } = {}) => {
		const caps = await reportCapabilities();
		return request<PlaybackPlan>(`/api/items/${encodeURIComponent(id)}/playback`, {
			query: { client: opts.client ?? 'web', network: opts.network, caps }
		});
	},

	/** Quality ladder and preferred delivery for the player menu. */
	options: () => request<PlaybackOptions>('/api/playback/options'),

	streamUrl: (id: string, token: string | null) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/stream`, token),

	startTranscode: (id: string, selection: TranscodeSelection = {}) =>
		request<TranscodeStatus>(`/api/items/${encodeURIComponent(id)}/transcode`, {
			method: 'POST',
			body: selection
		}),

	transcodeStatus: (id: string, session: string, signal?: AbortSignal) =>
		request<TranscodeStatus>(`/api/items/${encodeURIComponent(id)}/transcode/status`, {
			query: { session },
			signal
		}),

	/** Stop a session this client started (leaving for good, quality switch). */
	cancelTranscode: (id: string, session: string) =>
		request<TranscodeStatus>(`/api/items/${encodeURIComponent(id)}/transcode`, {
			method: 'DELETE',
			query: { session }
		}),

	transcodeUrl: (id: string, token: string | null, session?: string) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/transcode`, token, session ? { session } : {}),

	/** HLS playlist/segment URL (index.m3u8, init.mp4, segNNNNN.m4s). */
	hlsUrl: (id: string, token: string | null, session: string, file: string) =>
		mediaUrl(
			`/api/items/${encodeURIComponent(id)}/transcode/hls/${encodeURIComponent(file)}`,
			token,
			{ session }
		),

	subtitleUrl: (id: string, token: string | null, session: string) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/subtitles`, token, { session }),

	/**
	 * On-the-fly subtitle extraction: serves a WebVTT sidecar for one
	 * track of an item that is playing directly, so subtitles do not
	 * require a transcode (Jellyfin's "allow subtitle extraction on the
	 * fly").
	 */
	streamSubtitleUrl: (id: string, token: string | null, stream: number) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/subtitles`, token, { stream: String(stream) })
};
