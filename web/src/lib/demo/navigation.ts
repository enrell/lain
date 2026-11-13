// The demo build's stand-in for `$app/navigation` (see vite.config.ts).
//
// The demo routes by hash under a sub-path, while the client writes its
// routes as absolute paths (`goto('/library')`). Every path the client
// hands to the router is turned into its hash form here, so no caller
// changes.
import {
	afterNavigate as kitAfterNavigate,
	beforeNavigate as kitBeforeNavigate,
	goto as kitGoto,
	onNavigate as kitOnNavigate,
	preloadCode as kitPreloadCode,
	preloadData as kitPreloadData,
	pushState as kitPushState,
	replaceState as kitReplaceState
} from '$app/navigation';
import type { NavigationTarget } from '@sveltejs/kit';
import { hashed, unwrapHashUrl } from './url';

export * from '$app/navigation';
export { hashed };

function wrapTarget<T extends NavigationTarget | null>(target: T): T {
	if (!target || !target.url) return target;
	return { ...target, url: unwrapHashUrl(target.url) };
}

export const goto: typeof kitGoto = (url, opts) => kitGoto(hashed(url), opts);
export const preloadData: typeof kitPreloadData = (href) => kitPreloadData(hashed(href) as string);
export const preloadCode: typeof kitPreloadCode = (pathname) => kitPreloadCode(pathname);
export const pushState: typeof kitPushState = (url, state) => kitPushState(hashed(url), state);
export const replaceState: typeof kitReplaceState = (url, state) => kitReplaceState(hashed(url), state);

export const beforeNavigate: typeof kitBeforeNavigate = (cb) =>
	kitBeforeNavigate((nav) =>
		cb({
			...nav,
			from: wrapTarget(nav.from),
			to: wrapTarget(nav.to)
		})
	);

export const afterNavigate: typeof kitAfterNavigate = (cb) =>
	kitAfterNavigate((nav) =>
		cb({
			...nav,
			from: wrapTarget(nav.from),
			to: wrapTarget(nav.to)
		})
	);

export const onNavigate: typeof kitOnNavigate = (cb) =>
	kitOnNavigate((nav) =>
		cb({
			...nav,
			from: wrapTarget(nav.from),
			to: wrapTarget(nav.to)
		})
	);
