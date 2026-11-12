import { parseRecent, withRecent } from '$lib/utilities/search';

/*
 * The last searches that were meant (sent with Enter, or followed into a
 * result), kept in this browser: typing passes through too many half
 * words. One list for the header's field and the /search page, so a
 * search sent from either is offered again by both.
 */
const KEY = 'lain.search.recent';

function load(): string[] {
	try {
		return parseRecent(localStorage.getItem(KEY));
	} catch {
		// A browser that keeps nothing starts with none.
		return [];
	}
}

class RecentSearches {
	// Read once when the app starts (the web UI is a browser-only SPA): a
	// read inside a template must never write state.
	items = $state<string[]>(load());

	remember(term: string): void {
		this.items = withRecent(this.items, term);
		try {
			localStorage.setItem(KEY, JSON.stringify(this.items));
		} catch {
			// A browser that keeps nothing still searches.
		}
	}

	forget(): void {
		this.items = [];
		try {
			localStorage.removeItem(KEY);
		} catch {
			// Nothing was kept.
		}
	}
}

export const recentSearches = new RecentSearches();
