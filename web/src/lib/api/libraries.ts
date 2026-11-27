import { t } from '$lib/i18n';
import { request } from './client';
import { holdLease, syncLease } from './lease';
import type { Audience, BrowseResult, Library, LibraryRoot, ScanStatus } from './types';

/** Effective audience level (`everyone` when omitted). */
export function libraryAudience(lib: Pick<Library, 'audience'>): Audience {
	return lib.audience ?? 'everyone';
}

/**
 * Only libraries whose real name this session may see (filters out locked
 * private libraries, which have no name per D-293).
 */
export function namedLibraries(list: Library[]): Library[] {
	return list.filter((lib) => !isHidden(lib) && lib.name.trim().length > 0);
}

/** Deduplicated member ids of a private library with its owner first. */
export function effectiveMembers(lib: Pick<Library, 'owner' | 'members'>): string[] {
	const out: string[] = [];
	if (lib.owner) out.push(lib.owner);
	for (const m of lib.members ?? []) {
		if (m && !out.includes(m)) out.push(m);
	}
	return out;
}

/**
 * A locked private library comes without its name or folder (D-293).
 * Every screen shows it under one neutral label instead of a blank.
 */
export function shown(lib: Library): Library {
	return lib.locked && !lib.name ? { ...lib, name: t('privateLibrary.hiddenName') } : lib;
}

/** Whether a library's real name and folder are hidden from this session. */
export function isHidden(lib: Library): boolean {
	return !!lib.private && !!lib.locked;
}

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
	audience?: Audience;
	/** D-302: `true` turns "Fetch metadata online" off. */
	local_metadata?: boolean;
}

/** Unlock with the account's PIN or its password (D-156). */
export type UnlockInput = { pin: string } | { password: string };

/**
 * D-293: changing or removing a locked private library takes the PIN.
 * A session that unlocked it sends nothing; any other sends its own.
 */
export type Secret = UnlockInput | undefined;

export interface CreateLibraryInput {
	name: string;
	type: string;
	path: string;
	/** Left out, the server makes a new library `mature` (D-299). */
	audience?: Audience;
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
	list: async () => {
		const libs = (await request<Library[]>('/api/libraries')).map(shown);
		// What is unlocked holds only while this tab is seen alive (D-294).
		syncLease(libs);
		return libs;
	},

	create: (input: CreateLibraryInput) =>
		request<Library>('/api/libraries', { method: 'POST', body: input }),

	remove: (id: string, secret?: Secret) =>
		request<{ status: string }>(`/api/libraries/${encodeURIComponent(id)}`, {
			method: 'DELETE',
			...(secret ? { body: secret } : {})
		}),

	/** Starts a scan; while one runs the server queues one follow-up (D-286). */
	scanStart: () => request<{ state: string; queued?: boolean }>('/api/library/scan', { method: 'POST' }),

	/** The admin's order: every library id, first to last. */
	order: async (ids: string[]) =>
		(await request<Library[]>('/api/libraries/order', { method: 'PUT', body: { ids } })).map(shown),

	scanStatus: () => request<ScanStatus>('/api/library/scan'),

	/** Starts the metadata pass (D-302); `retry` asks again about titles no provider knew. */
	enrichStart: (retry = false) =>
		request<{ state: string; queued?: boolean }>('/api/library/enrich', { method: 'POST', body: { retry } }),

	update: async (id: string, patch: LibraryPatch, secret?: Secret) =>
		shown(
			await request<Library>(`/api/libraries/${encodeURIComponent(id)}`, {
				method: 'PATCH',
				body: { ...patch, ...secret }
			})
		),

	/** Unlocks a private library for this session only. */
	unlock: async (id: string, input: UnlockInput) => {
		const lib = shown(
			await request<Library>(`/api/libraries/${encodeURIComponent(id)}/unlock`, { method: 'POST', body: input })
		);
		holdLease();
		return lib;
	},

	lock: async (id: string) =>
		shown(await request<Library>(`/api/libraries/${encodeURIComponent(id)}/lock`, { method: 'POST' })),

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
