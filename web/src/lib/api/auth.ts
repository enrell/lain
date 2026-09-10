import { request } from './client';
import type { LoginResponse, SetupStatus, User } from './types';

/** Gateway routes: GET /api/setup/status, POST /api/setup, POST /api/auth/login. */
export const auth = {
	setupStatus: () => request<SetupStatus>('/api/setup/status'),

	setup: (username: string, password: string) =>
		request<User>('/api/setup', { method: 'POST', body: { username, password } }),

	login: (username: string, password: string) =>
		request<LoginResponse>('/api/auth/login', { method: 'POST', body: { username, password } })
};
