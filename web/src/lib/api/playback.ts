import { mediaUrl, request } from './client';
import type { PlaybackPlan, TranscodeSelection, TranscodeStatus } from './types';

/**
 * Gateway routes: GET /api/items/{id}/playback (plan),
 * GET /api/items/{id}/stream (bytes, Range-capable) and
 * GET /api/items/{id}/transcode (prepared MP4, Range-capable).
 *
 * The plan is the playback policy: the UI must honor `available` and
 * `mode` instead of reconstructing URLs by itself. A `transcode` plan
 * plays the transcode URL; anything else playable plays the stream
 * URL. Both media URLs are the only place a token rides in the query
 * string — native media elements cannot send Authorization headers.
 */
export const playback = {
	plan: (id: string, opts: { client?: string; network?: string } = {}) =>
		request<PlaybackPlan>(`/api/items/${encodeURIComponent(id)}/playback`, {
			query: { client: opts.client ?? 'web', network: opts.network }
		}),

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

	transcodeUrl: (id: string, token: string | null, session?: string) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/transcode`, token, session ? { session } : {}),

	subtitleUrl: (id: string, token: string | null, session: string) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/subtitles`, token, { session })
};
