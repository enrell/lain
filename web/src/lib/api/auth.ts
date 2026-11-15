import { request } from './client';
import type { LoginResponse, SetupStatus, User } from './types';

/** Gateway routes: GET /api/setup/status, POST /api/setup, POST /api/auth/login,
 * POST /api/auth/logout. */
export const auth = {
	setupStatus: () => request<SetupStatus>('/api/setup/status'),

	setup: (username: string, password: string) =>
		request<User>('/api/setup', { method: 'POST', body: { username, password } }),

	login: (username: string, password: string) =>
		request<LoginResponse>('/api/auth/login', { method: 'POST', body: { username, password } }),

	/**
	 * Ends the session of `token` on the server (D-298). The token is
	 * passed in because the session has already dropped it; `keepalive`
	 * lets the request outlive the page.
	 */
	logout: (token: string) =>
		request<{ signed_out: boolean }>('/api/auth/logout', {
			method: 'POST',
			token,
			keepalive: true,
			suppressAuthRedirect: true
		})
};
