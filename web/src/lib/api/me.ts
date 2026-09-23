import { request } from './client';
import type { Progress, User } from './types';

/** Gateway routes: GET /api/me, PATCH /api/me/password, GET /api/me/continue. */
export const me = {
	get: () => request<User>('/api/me'),
	setPreferredLanguage: (preferredLanguage: string) =>
		request<User>('/api/me/preferences', {
			method: 'PATCH',
			body: { preferred_language: preferredLanguage }
		}),

	changePassword: (oldPassword: string, newPassword: string) =>
		request<{ status: string }>('/api/me/password', {
			method: 'PATCH',
			body: { old: oldPassword, new: newPassword }
		}),

	/** All of the user's progress records (unsorted; the UI sorts). */
	continueWatching: () => request<Progress[]>('/api/me/continue')
};
