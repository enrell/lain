import { request } from './client';
import type { DownloadJob, DownloadSettings, DownloadsView, DownloadCleanup } from './types';

/**
 * Gateway routes (admin): the server download manager. Jobs fetch a URL
 * onto server disk — into a library root (then rescanned) or the
 * configured download directory — within the operator's byte budget
 * and free-space floor. Errors carry stable codes: `quota-exceeded`,
 * `disk-full`, `invalid-request`, `invalid-state`, `not-found`.
 */
export const downloads = {
	list: () => request<DownloadsView>('/api/downloads'),

	add: (input: { url: string; name?: string; library_id?: string }) =>
		request<DownloadJob>('/api/downloads', { method: 'POST', body: input }),

	action: (id: string, action: 'pause' | 'resume' | 'cancel') =>
		request<DownloadJob>(`/api/downloads/${encodeURIComponent(id)}/${action}`, { method: 'POST' }),

	remove: (id: string) =>
		request<{ status: string }>(`/api/downloads/${encodeURIComponent(id)}`, { method: 'DELETE' }),

	saveSettings: (settings: DownloadSettings) =>
		request<DownloadSettings>('/api/downloads/settings', { method: 'PUT', body: settings }),

	cleanup: () => request<DownloadCleanup>('/api/downloads/cleanup', { method: 'POST' })
};
