import { request } from './client';
import type {
	AuthorizeResponse,
	IntegrationsResponse,
	LinkedAccountView,
	LinksResponse,
	ListResponse,
	SyncResponse
} from './types';

/**
 * Gateway routes: GET /api/me/links, {platform}/authorize + /sync,
 * DELETE /api/me/links/{platform}, GET /api/list.
 */
export const links = {
	list: () => request<LinksResponse>('/api/me/links'),

	/** The OAuth URL the user opens on the platform. */
	authorize: (platform: string) =>
		request<AuthorizeResponse>(`/api/me/links/${encodeURIComponent(platform)}/authorize`),

	/** The pin-flow URL whose page hands the user an authorization code
	 * to paste back — zero server setup (D-083). */
	pin: (platform: string) =>
		request<AuthorizeResponse>(`/api/me/links/${encodeURIComponent(platform)}/pin`),

	/** Exchange the code the pin page displayed. */
	code: (platform: string, code: string) =>
		request<{ link: LinkedAccountView }>(`/api/me/links/${encodeURIComponent(platform)}/code`, {
			method: 'POST',
			body: { code }
		}),

	/** Opt in or out of pushing watch progress to the platform. */
	setScrobble: (platform: string, scrobble: boolean) =>
		request<LinkedAccountView>(`/api/me/links/${encodeURIComponent(platform)}`, {
			method: 'PATCH',
			body: { scrobble }
		}),

	unlink: (platform: string) =>
		request<{ status: string; removed: number }>(`/api/me/links/${encodeURIComponent(platform)}`, {
			method: 'DELETE'
		}),

	sync: (platform: string) =>
		request<SyncResponse>(`/api/me/links/${encodeURIComponent(platform)}/sync`, {
			method: 'POST'
		})
};

/** The user's unified tracking list (D-078). */
export const list = {
	entries: (filters: { type?: string; status?: string } = {}) =>
		request<ListResponse>('/api/list', { query: filters })
};

/** Admin integration credentials (D-080); the secret never comes back. */
export const integrations = {
	settings: () => request<IntegrationsResponse>('/api/admin/settings/integrations'),

	save: (body: { anilist_client_id?: string; anilist_client_secret?: string }) =>
		request<IntegrationsResponse>('/api/admin/settings/integrations', {
			method: 'PUT',
			body
		})
};
