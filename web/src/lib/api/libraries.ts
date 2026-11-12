import { request } from './client';
import type { BrowseResult, Library, LibraryRoot, ScanStatus } from './types';

/** An admin change (lain-rs): rename, type, folder, privacy, owner and members. */
export interface LibraryPatch {
	name?: string;
	/** Which files the next scan reads (`auto`, `anime`, `movie`, `manga`...). */
	type?: string;
	/** A new folder on the server; the id, progress and overlays stay. */
	path?: string;
	private?: boolean;
	owner?: string;
	members?: string[];
}

/** Unlock with the account's PIN or its password (D-156). */
export type UnlockInput = { pin: string } | { password: string };

export interface CreateLibraryInput {
	name: string;
	type: string;
	path: string;
}

/**
 * Gateway routes: GET/POST /api/libraries, DELETE /api/libraries/{id},
 * POST/GET /api/library/scan. lain-rs adds PATCH /api/libraries/{id},
 * /unlock, /lock, /api/me/pin and /api/roots (D-156, D-157).
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

	/** Starts a scan; while one runs the server queues one follow-up (D-286). */
	scanStart: () => request<{ state: string; queued?: boolean }>('/api/library/scan', { method: 'POST' }),

	/** The admin's order: every library id, first to last. */
	order: (ids: string[]) => request<Library[]>('/api/libraries/order', { method: 'PUT', body: { ids } }),

	scanStatus: () => request<ScanStatus>('/api/library/scan'),

	update: (id: string, patch: LibraryPatch) =>
		request<Library>(`/api/libraries/${encodeURIComponent(id)}`, { method: 'PATCH', body: patch }),

	/** Unlocks a private library for this session only. */
	unlock: (id: string, input: UnlockInput) =>
		request<Library>(`/api/libraries/${encodeURIComponent(id)}/unlock`, { method: 'POST', body: input }),

	lock: (id: string) =>
		request<Library>(`/api/libraries/${encodeURIComponent(id)}/lock`, { method: 'POST' }),

	/** lain-rs only; the Go server answers 404, which callers treat as "no roots". */
	roots: () => request<{ format: string; roots: LibraryRoot[] }>('/api/roots'),

	pin: {
		get: () => request<{ set: boolean }>('/api/me/pin'),
		set: (pin: string, password: string) =>
			request<{ set: boolean }>('/api/me/pin', { method: 'PUT', body: { pin, password } }),
		clear: (password: string) =>
			request<{ set: boolean }>('/api/me/pin', { method: 'DELETE', body: { password } })
	},

	browse: (path?: string) =>
		request<BrowseResult>(path ? `/api/browse?path=${encodeURIComponent(path)}` : '/api/browse')
};
