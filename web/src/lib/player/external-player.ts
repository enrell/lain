export type ExternalPlayer = 'mpv' | 'vlc';

/** Local protocol handoff. Authentication stays in the installed CLI. */
export function externalPlayerUrl(origin: string, itemId: string, player: ExternalPlayer): string {
	const query = new URLSearchParams({ server: origin, id: itemId, player });
	return `lain://play?${query}`;
}
