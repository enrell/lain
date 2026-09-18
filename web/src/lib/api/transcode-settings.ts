import { request } from './client';
import type {
	TranscodeSettings,
	TranscodeSettingsResponse,
	TranscodeSessionsResponse,
	TranscodeStatus
} from './types';

/**
 * Gateway routes (admin): GET/PUT /api/admin/settings/transcode and
 * GET /api/admin/transcodes, DELETE /api/admin/transcodes/{session}.
 * Settings apply to new sessions without a restart; the response
 * carries the probed capabilities so the UI can show what the local
 * ffmpeg actually supports (D-031/D-045).
 */
export const transcodeAdmin = {
	settings: () => request<TranscodeSettingsResponse>('/api/admin/settings/transcode'),

	saveSettings: (settings: TranscodeSettings) =>
		request<TranscodeSettingsResponse>('/api/admin/settings/transcode', {
			method: 'PUT',
			body: settings
		}),

	sessions: () => request<TranscodeSessionsResponse>('/api/admin/transcodes'),

	cancelSession: (session: string) =>
		request<TranscodeStatus>(`/api/admin/transcodes/${encodeURIComponent(session)}`, {
			method: 'DELETE'
		})
};
