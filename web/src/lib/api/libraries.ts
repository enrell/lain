import { request } from './client';
import type { Library, ScanStatus } from './types';

export interface CreateLibraryInput {
	name: string;
	type: string;
	path: string;
}

/**
 * Gateway routes: GET/POST /api/libraries, DELETE /api/libraries/{id},
 * POST/GET /api/library/scan.
 *
 * `path` is a directory on the *server's* filesystem, not the
 * browser's; the UI must make that distinction explicit.
 */
export const libraries = {
	list: () => request<Library[]>('/api/libraries'),

	create: (input: CreateLibraryInput) =>
		request<Library>('/api/libraries', { method: 'POST', body: input }),

	remove: (id: string) =>
		request<{ status: string }>(`/api/libraries/${encodeURIComponent(id)}`, {
			method: 'DELETE'
		}),

	scanStart: () => request<{ state: string }>('/api/library/scan', { method: 'POST' }),

	scanStatus: () => request<ScanStatus>('/api/library/scan')
};
