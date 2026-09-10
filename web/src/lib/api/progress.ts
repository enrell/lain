import { request } from './client';
import type { Progress } from './types';

export interface ProgressWrite {
	position_sec: number;
	duration_sec: number;
	completed?: boolean;
}

/** Gateway routes: GET/PUT /api/items/{id}/progress. */
export const progress = {
	get: (id: string) =>
		request<Progress>(`/api/items/${encodeURIComponent(id)}/progress`),

	put: (id: string, write: ProgressWrite, opts: { keepalive?: boolean } = {}) =>
		request<Progress>(`/api/items/${encodeURIComponent(id)}/progress`, {
			method: 'PUT',
			body: write,
			keepalive: opts.keepalive
		})
};
