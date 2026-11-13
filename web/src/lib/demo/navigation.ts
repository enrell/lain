// The demo build's stand-in for `$app/navigation` (see vite.config.ts).
//
// The demo routes by hash under a sub-path, while the client writes its
// routes as absolute paths (`goto('/library')`). Every path the client
// hands to the router is turned into its hash form here, so no caller
// changes.
import {
	goto as kitGoto,
	preloadCode as kitPreloadCode,
	preloadData as kitPreloadData,
	pushState as kitPushState,
	replaceState as kitReplaceState
} from '$app/navigation';

export * from '$app/navigation';

/** `/library?x=1` as `#/library?x=1`; anything else as it came. */
export function hashed<T extends string | URL>(url: T): T | string {
	if (typeof url === 'string') return url.startsWith('/') && !url.startsWith('//') ? `#${url}` : url;
	return url;
}

export const goto: typeof kitGoto = (url, opts) => kitGoto(hashed(url), opts);
export const preloadData: typeof kitPreloadData = (href) => kitPreloadData(hashed(href) as string);
export const preloadCode: typeof kitPreloadCode = (pathname) => kitPreloadCode(pathname);
export const pushState: typeof kitPushState = (url, state) => kitPushState(hashed(url), state);
export const replaceState: typeof kitReplaceState = (url, state) => kitReplaceState(hashed(url), state);
