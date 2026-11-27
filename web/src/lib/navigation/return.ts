import { goto } from '$app/navigation';

/**
 * Where "back" goes around the player and the reader, so neither is ever
 * a page you land on by going back.
 *
 * Leaving the player for the title page used to open that page anew: the
 * history then read library → title → player → title, and both the title
 * page's back arrow and the browser's back button led into the player
 * again. Now the player (and the reader) goes back in history when it was
 * opened from the page it returns to, and otherwise replaces its own
 * entry; and the title page's back arrow, reached from a player, goes to
 * the last list page (home, a library, search) instead of to the player.
 */

/** A page that plays or reads a file: never a place "back" should land. */
export function isPlayback(path: string): boolean {
	return path.startsWith('/player/') || path.startsWith('/read/');
}

/** A title page. */
function isTitle(path: string): boolean {
	return path.startsWith('/item/');
}

/** The in-app page the current player or reader was opened from (path and query). */
let playbackFrom: string | null = null;
/** The last page that was neither a title, a player nor a reader. */
let lastList: string | null = null;

/** Called after every navigation (the root layout). */
export function noteNavigation(from: URL | null | undefined, to: URL | null | undefined): void {
	if (!to) return;
	const toPath = to.pathname;
	if (isPlayback(toPath)) {
		// The next episode replaces the player: keep where the first one came from.
		if (!from || !isPlayback(from.pathname)) playbackFrom = from ? from.pathname + from.search : null;
		return;
	}
	playbackFrom = null;
	if (!isTitle(toPath)) lastList = toPath + to.search;
}

/**
 * Leaves the player or reader for `detailHref` (the title page): back in
 * history when that is where it was opened from, else the title page in
 * place of this entry.
 */
export function leavePlayback(detailHref: string): void {
	if (playbackFrom !== null && isTitle(playbackFrom) && typeof history !== 'undefined' && history.length > 1) {
		history.back();
		return;
	}
	void goto(detailHref, { replaceState: true });
}

/** Moves to another file in the player or reader without stacking an entry per file. */
export function playbackTo(href: string): Promise<void> {
	return goto(href, { replaceState: true });
}

/** The title page's way back when the page before it was a player: the last list page. */
export function listFallback(): string {
	return lastList ?? '/library';
}

/** Tests only. */
export function resetNavigation(): void {
	playbackFrom = null;
	lastList = null;
}
