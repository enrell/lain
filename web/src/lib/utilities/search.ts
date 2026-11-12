/*
 * Rules the header's search field and the /search page share, so the two
 * cannot disagree about where a query leads or which searches are kept.
 */

/** How many meant searches a browser keeps. */
export const RECENT_LIMIT = 6;

/** The results page for a query; the bare page (recents, browse) for none. */
export function searchHref(query: string): string {
	const term = query.trim();
	return term === '' ? '/search' : `/search?q=${encodeURIComponent(term)}`;
}

/**
 * The list with `term` first and only once, whatever its case. A term
 * under two letters is a half word, not a search worth offering again.
 */
export function withRecent(list: string[], term: string, limit = RECENT_LIMIT): string[] {
	const q = term.trim();
	if (q.length < 2) return list;
	return [q, ...list.filter((r) => r.toLowerCase() !== q.toLowerCase())].slice(0, limit);
}

/** A kept list read back; anything that is not a list of strings is none. */
export function parseRecent(raw: string | null, limit = RECENT_LIMIT): string[] {
	try {
		const kept: unknown = JSON.parse(raw ?? '[]');
		return Array.isArray(kept) ? kept.filter((r): r is string => typeof r === 'string').slice(0, limit) : [];
	} catch {
		return [];
	}
}
