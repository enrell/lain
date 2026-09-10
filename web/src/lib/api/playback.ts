import { mediaUrl, request } from './client';
import type { PlaybackPlan } from './types';

/**
 * Gateway routes: GET /api/items/{id}/playback (plan) and
 * GET /api/items/{id}/stream (bytes, Range-capable).
 *
 * The plan is the playback policy: the UI must honor `available` and
 * `mode` instead of reconstructing URLs by itself. The stream URL is
 * the only place a token rides in the query string — native media
 * elements cannot send Authorization headers.
 */
export const playback = {
	plan: (id: string, opts: { client?: string; network?: string } = {}) =>
		request<PlaybackPlan>(`/api/items/${encodeURIComponent(id)}/playback`, {
			query: { client: opts.client ?? 'web', network: opts.network }
		}),

	streamUrl: (id: string, token: string | null) =>
		mediaUrl(`/api/items/${encodeURIComponent(id)}/stream`, token)
};
