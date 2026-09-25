import { ApiError, api } from '$lib/api';
import { prefs } from '$lib/auth/storage';
import { toasts } from '$lib/stores/toasts.svelte';
import { errorMessage } from '$lib/utilities/errors';

export type ExternalPlayer = 'mpv' | 'vlc';
export type PreferredPlayer = 'browser' | ExternalPlayer;

export function loadPreferredPlayer(userId: string): PreferredPlayer {
	const value = prefs.get(`player.${userId}`);
	// 'local' was the pre-autodetect spelling of "the player on this
	// machine" (D-072); it maps to the default local player.
	if (value === 'local') return 'mpv';
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

let localPlayersCache: Promise<string[]> | null = null;

/**
 * Players the server can launch itself (D-072): non-empty only when
 * this browser reached the server over loopback, i.e. same machine.
 * Cached once per page load — a machine does not gain players mid-click.
 */
function localPlayers(): Promise<string[]> {
	localPlayersCache ??= api.localplay
		.players()
		.then((cap) => cap.players ?? [])
		.catch(() => []);
	return localPlayersCache;
}

/**
 * Click intercept for Play buttons. When the server is on this machine
 * it launches the player itself; anywhere else the click is replayed
 * against its original lain:// destination (D-070). preventDefault runs
 * synchronously because the fallback re-navigates by hand.
 */
export async function playExternalClick(
	event: MouseEvent,
	itemId: string,
	preferred: PreferredPlayer
): Promise<void> {
	if (preferred === 'browser') return;
	const href = (event.currentTarget as HTMLAnchorElement).href;
	event.preventDefault();
	const players = await localPlayers();
	if (players.length > 0) {
		// When the browser is local, the server's capability list is the
		// machine's player list — a missing binary would fail the lain://
		// path identically, so the first available player is correct.
		const player = players.includes(preferred) ? preferred : players[0];
		try {
			const result = await api.localplay.play(itemId, player);
			toasts.success(`Playing in ${result.player} on this machine.`);
			return;
		} catch (err) {
			if (!(err instanceof ApiError) || (err.kind !== 'forbidden' && err.kind !== 'unavailable')) {
				toasts.error(errorMessage(err, 'Could not start playback.'));
				return;
			}
		}
	}
	window.location.assign(href);
}
