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

	/** PATCH /api/me/profile: only the fields present change. mascot '' clears the avatar. */
	updateProfile: (patch: { display_name?: string; bio?: string; mascot?: string }) =>
		request<User>('/api/me/profile', { method: 'PATCH', body: patch }),

	/** PUT /api/me/avatar: PNG, JPEG, GIF or WebP up to 2 MB, checked by content on the server. */
	uploadAvatar: (file: Blob) => request<User>('/api/me/avatar', { method: 'PUT', rawBody: file }),

	removeAvatar: () => request<User>('/api/me/avatar', { method: 'DELETE' }),

	/** All of the user's progress records (unsorted; the UI sorts). */
	continueWatching: () => request<Progress[]>('/api/me/continue')
};
