import { request } from './client';
import type { Audience, User, UserPlaybackPolicy } from './types';

export interface CreateUserInput {
	username: string;
	password: string;
	role?: string;
}

export interface PatchUserInput {
	disabled?: boolean;
	role?: string;
	password?: string;
	/** Per-user playback limits (D-042). */
	playback?: UserPlaybackPolicy;
	/** Explicit library allow-list (`null` clears back to all libraries). */
	libraries?: string[] | null;
	/** Highest audience level (`everyone` | `teen` | `mature`). */
	max_audience?: Audience;
}

/** What a deleted account took with it. */
export interface UserDeleted {
	status: string;
	rows_removed: number;
	/** Private libraries it owned, now the deleting admin's. */
	libraries_reassigned: string[];
}

/** Effective audience ceiling (`mature` when omitted). */
export function userMaxAudience(user: Pick<User, 'max_audience'>): Audience {
	return user.max_audience ?? 'mature';
}

/**
 * Computes the next `libraries` allow-list when toggling `targetId` for an
 * account. When `current` is unset ("all libraries"), starts from the
 * currently visible named libraries so toggling one off excludes only it.
 */
export function toggleUserLibrary(
	current: string[] | undefined,
	visibleIds: string[],
	targetId: string
): string[] {
	const base = current ?? visibleIds;
	return base.includes(targetId) ? base.filter((id) => id !== targetId) : [...base, targetId];
}

/** Gateway routes: GET/POST /api/users, PATCH and DELETE /api/users/{id} (admin). */
export const users = {
	list: () => request<User[]>('/api/users'),

	create: (input: CreateUserInput) =>
		request<User>('/api/users', { method: 'POST', body: input }),

	patch: (id: string, input: PatchUserInput) =>
		request<User>(`/api/users/${encodeURIComponent(id)}`, { method: 'PATCH', body: input }),

	remove: (id: string) => request<UserDeleted>(`/api/users/${encodeURIComponent(id)}`, { method: 'DELETE' })
};
