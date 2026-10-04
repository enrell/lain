/*
 * What the service worker may answer from the offline cache, kept
 * separate from the worker so it is unit-testable. Every rule is
 * network-first: a cached copy is used only when the network fails, so
 * online behavior is exactly what it was without a worker.
 */

/** Cache holding saved items, their reads and the session reads. */
export const OFFLINE_CACHE = 'lain-offline-v1';
/** Key of the saved-items index inside OFFLINE_CACHE. */
export const INDEX_KEY = '/__lain-offline__/index.json';

export type Route = 'navigate' | 'asset' | 'saved' | 'session' | 'pass';

const SAVED = [
	/^\/api\/items\/[^/]+\/pages$/,
	/^\/api\/items\/[^/]+\/pages\/\d+$/,
	/^\/api\/items\/[^/]+\/progress$/,
	/^\/api\/catalog\/[^/]+$/,
	/^\/api\/catalog\/[^/]+\/episodes$/
];
const SESSION = new Set(['/api/setup/status', '/api/me']);

/**
 * Classifies a request. `assets` are the build's own files (precached
 * app shell). Only same-origin GETs are ever touched.
 */
export function classify(
	url: URL,
	method: string,
	mode: string,
	origin: string,
	assets: ReadonlySet<string>
): Route {
	if (method !== 'GET' || url.origin !== origin) return 'pass';
	if (mode === 'navigate' && !url.pathname.startsWith('/api/')) return 'navigate';
	if (assets.has(url.pathname)) return 'asset';
	if (SESSION.has(url.pathname)) return 'session';
	if (SAVED.some((re) => re.test(url.pathname))) return 'saved';
	return 'pass';
}

/**
 * The cache key of a request: path plus query without the auth token,
 * so a page image saved with one token is found with the next one.
 */
export function cacheKey(url: URL): string {
	const q = new URLSearchParams(url.search);
	q.delete('token');
	const s = q.toString();
	return s ? `${url.pathname}?${s}` : url.pathname;
}

/** Every key an item occupies once saved. */
export function itemKeys(itemId: string, pages: number): string[] {
	const id = encodeURIComponent(itemId);
	const keys = [
		`/api/items/${id}/pages`,
		`/api/items/${id}/progress`,
		`/api/catalog/${id}`,
		`/api/catalog/${id}/episodes`
	];
	for (let i = 0; i < pages; i++) keys.push(`/api/items/${id}/pages/${i}`);
	return keys;
}
