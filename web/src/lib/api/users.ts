import { request } from './client';
import type { User } from './types';

export interface CreateUserInput {
	username: string;
	password: string;
	role?: 'admin' | 'user';
}

export interface PatchUserInput {
	disabled?: boolean;
	role?: 'admin' | 'user';
	password?: string;
}

/** Gateway routes: GET/POST /api/users, PATCH /api/users/{id} (admin). */
export const users = {
	list: () => request<User[]>('/api/users'),

	create: (input: CreateUserInput) =>
		request<User>('/api/users', { method: 'POST', body: input }),

	patch: (id: string, input: PatchUserInput) =>
		request<User>(`/api/users/${encodeURIComponent(id)}`, { method: 'PATCH', body: input })
};
