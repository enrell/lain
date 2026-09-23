export type ExternalPlayer = 'mpv' | 'vlc';
export type PreferredPlayer = 'browser' | ExternalPlayer;

import { prefs } from '$lib/auth/storage';

export function loadPreferredPlayer(userId: string): PreferredPlayer {
	const value = prefs.get(`player.${userId}`);
	return value === 'mpv' || value === 'vlc' ? value : 'browser';
}

export function savePreferredPlayer(userId: string, player: PreferredPlayer): void {
	prefs.set(`player.${userId}`, player);
}

export function playbackHref(origin: string, itemId: string, player: PreferredPlayer): string {
	return player === 'browser' || !origin
		? `/player/${encodeURIComponent(itemId)}`
		: externalPlayerUrl(origin, itemId, player);
}

/** Local protocol handoff. Authentication stays in the installed CLI. */
export function externalPlayerUrl(origin: string, itemId: string, player: ExternalPlayer): string {
	const query = new URLSearchParams({ server: origin, id: itemId, player });
	return `lain://play?${query}`;
}
