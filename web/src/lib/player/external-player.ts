import { goto } from '$app/navigation';
import { ApiError, api } from '$lib/api';
import { prefs } from '$lib/auth/storage';
import { toasts } from '$lib/stores/toasts.svelte';
import { errorMessage } from '$lib/utilities/errors';

export type ExternalPlayer = 'mpv' | 'vlc';
export type PreferredPlayer = 'browser' | 'local' | ExternalPlayer;

export function loadPreferredPlayer(userId: string): PreferredPlayer {
	const value = prefs.get(`player.${userId}`);
	return value === 'mpv' || value === 'vlc' || value === 'local' ? value : 'browser';
}

export function savePreferredPlayer(userId: string, player: PreferredPlayer): void {
	prefs.set(`player.${userId}`, player);
}

export function playbackHref(origin: string, itemId: string, player: PreferredPlayer): string {
	// "local" keeps the in-browser href: Play buttons intercept the click
	// and POST to the server, so a bare navigation (new tab, middle
	// click) still lands on the web player.
	return player === 'browser' || player === 'local' || !origin
		? `/player/${encodeURIComponent(itemId)}`
		: externalPlayerUrl(origin, itemId, player);
}

/** Local protocol handoff. Authentication stays in the installed CLI. */
export function externalPlayerUrl(origin: string, itemId: string, player: ExternalPlayer): string {
	const query = new URLSearchParams({ server: origin, id: itemId, player });
	return `lain://play?${query}`;
}

/**
 * Click intercept for Play buttons (D-072). When the preference is
 * "local" the server launches the player on its own machine; failures
 * that mean the capability is gone (403 remote, 503 no player) fall
 * back to the web player, while busy/conflict surfaces a toast —
 * doubling playback would be worse than explaining it.
 */
export async function playLocallyOrBrowser(
	event: MouseEvent,
	itemId: string,
	preferred: PreferredPlayer
): Promise<void> {
	if (preferred !== 'local') return;
	event.preventDefault();
	try {
		const result = await api.localplay.play(itemId);
		toasts.success(`Playing in ${result.player} on this machine.`);
	} catch (err) {
		if (err instanceof ApiError && (err.kind === 'forbidden' || err.kind === 'unavailable')) {
			await goto(`/player/${encodeURIComponent(itemId)}`);
			return;
		}
		toasts.error(errorMessage(err, 'Could not start local playback.'));
	}
}
