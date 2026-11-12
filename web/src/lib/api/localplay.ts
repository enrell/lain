import { request } from './client';

export interface LocalPlayCapability {
	/** Player binaries usable on the server machine; `null` when there are
	 * none or the request is not from that machine (the server omits an
	 * empty list). */
	players: string[] | null;
}

export interface PlayLocalResult {
	status: string;
	player: string;
	count: number;
}

/**
 * Local playback (D-072): the server spawns mpv/VLC on its own machine
 * for loopback browsers — no token or protocol handoff involved. The
 * capability is honest: remote peers and playerless/Docker servers
 * report an empty list, and POST returns 403/503 instead.
 */
export const localplay = {
	players: () => request<LocalPlayCapability>('/api/localplay'),

	play: (id: string, player?: string) =>
		request<PlayLocalResult>(`/api/items/${encodeURIComponent(id)}/play-local`, {
			method: 'POST',
			body: player ? { player } : {}
		})
};
