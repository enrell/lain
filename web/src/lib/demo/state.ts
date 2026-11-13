// The demo build's stand-in for `$app/state` (see vite.config.ts).
// SvelteKit's hash router leaves `page.url.pathname` stuck at the mount path
// and pushes the real route/query into `page.url.hash`; this wrapper unwraps
// it reactively so every component reads canonical paths and search params.

import { navigating as kitNavigating, page as kitPage, updated as kitUpdated } from '$app/state';
import type { NavigationTarget } from '@sveltejs/kit';
import { unwrapHashUrl } from './url';

function wrapTarget(target: NavigationTarget | null): NavigationTarget | null {
	if (!target || !target.url) return target;
	return { ...target, url: unwrapHashUrl(target.url) };
}

export const page: typeof kitPage = {
	get data() {
		return kitPage.data;
	},
	get error() {
		return kitPage.error;
	},
	get form() {
		return kitPage.form;
	},
	get params() {
		return kitPage.params;
	},
	get route() {
		return kitPage.route;
	},
	get state() {
		return kitPage.state;
	},
	get status() {
		return kitPage.status;
	},
	get url() {
		return unwrapHashUrl(kitPage.url);
	}
};

export const navigating: typeof kitNavigating = {
	get from() {
		return wrapTarget(kitNavigating.from);
	},
	get to() {
		return wrapTarget(kitNavigating.to);
	},
	get type() {
		return kitNavigating.type;
	},
	get willUnload() {
		return kitNavigating.willUnload;
	},
	get delta() {
		return kitNavigating.delta;
	},
	get complete() {
		return kitNavigating.complete;
	}
} as typeof kitNavigating;

export const updated = kitUpdated;
