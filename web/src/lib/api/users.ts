import { request } from './client';
import type { User, UserPlaybackPolicy } from './types';

export interface CreateUserInput {
	username: string;
	password: string;
	role?: 'admin' | 'user';
}

export interface PatchUserInput {
	disabled?: boolean;
	role?: 'admin' | 'user';
	password?: string;
	/** Per-user playback limits (D-042). */
	playback?: UserPlaybackPolicy;
}

/** What a deleted account took with it. */
export interface UserDeleted {
	status: string;
	rows_removed: number;
	/** Private libraries it owned, now the deleting admin's. */
	libraries_reassigned: string[];
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
