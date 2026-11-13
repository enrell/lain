import { base } from '$app/paths';

const HASH_ROUTE_PREFIX = '#/';

function readHash(url: URL): string {
	try {
		return url.hash;
	} catch {
		return typeof window !== 'undefined' ? window.location.hash : '';
	}
}

function currentAppUrl(basePath: string): URL {
	if (typeof window !== 'undefined') {
		return unwrapHashUrl(new URL(window.location.href), basePath);
	}
	return new URL('http://lain.local/');
}

/**
 * Resolves a SvelteKit hash-router URL (`<origin>/<base>/#/path?q=1`) into an
 * app-level `URL` whose `pathname`, `search`, `searchParams`, and `hash`
 * match the client's logical routes.
 */
export function unwrapHashUrl<T extends URL | null | undefined>(
	raw: T,
	basePath: string = base
): T {
	if (!raw || raw.protocol === 'a:') return raw;

	const hash = readHash(raw);
	if (hash.startsWith(HASH_ROUTE_PREFIX)) {
		const route = new URL(hash.slice(1), raw.origin);
		if (!route.search && raw.search) {
			route.search = raw.search;
		}
		return route as T;
	}

	if (hash.startsWith('#?')) {
		return new URL(`/${hash.slice(1)}`, raw.origin) as T;
	}

	const normBase = basePath.replace(/\/$/, '');
	if (normBase && (raw.pathname === normBase || raw.pathname.startsWith(`${normBase}/`))) {
		const sub = raw.pathname.slice(normBase.length) || '/';
		return new URL(`${sub}${raw.search}${hash}`, raw.origin) as T;
	}

	return raw;
}

/**
 * Normalizes a `load({ url })` input for the demo build while leaving normal
 * server builds untouched.
 */
export function resolveLoadUrl(url: URL): URL {
	return import.meta.env.VITE_DEMO ? unwrapHashUrl(url) : url;
}

/**
 * Translates an app route target (`/library?view=all`, `?sort=title`, `#frag`,
 * or `new URL(...)`) into the `#/...` URL expected by SvelteKit's hash router.
 */
export function hashed<T extends string | URL>(
	url: T,
	currentRaw?: URL,
	basePath: string = base
): T | string {
	const current = currentRaw ? unwrapHashUrl(currentRaw, basePath) : currentAppUrl(basePath);

	if (url instanceof URL) {
		if (url.origin !== current.origin) return url;
		const hash = readHash(url);
		const normBase = basePath.replace(/\/$/, '');
		if (hash.startsWith(HASH_ROUTE_PREFIX)) {
			if (!normBase || url.pathname === normBase || url.pathname === `${normBase}/`) {
				return hash;
			}
		}
		const appUrl = unwrapHashUrl(url, basePath);
		return `#${appUrl.pathname}${appUrl.search}${appUrl.hash}`;
	}

	if (url.startsWith(HASH_ROUTE_PREFIX)) return url;
	if (url.startsWith('/') && !url.startsWith('//')) return `#${url}`;
	if (url.startsWith('?')) return `#${current.pathname}${url}`;
	if (url.startsWith('#')) return `#${current.pathname}${current.search}${url}`;
	return url;
}
