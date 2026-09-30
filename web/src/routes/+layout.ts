import { i18n } from '$lib/i18n';
import type { LayoutLoad } from './$types';

// Pure SPA: every route is rendered in the browser and the Go server
// serves the same static shell for any client-side path.
export const ssr = false;
export const prerender = false;
export const trailingSlash = 'never';

// The catalog must be in place before the first render, or the first
// frame would flash source-language text in a translated UI.
let ready: Promise<void> | null = null;
export const load: LayoutLoad = async () => {
	ready ??= i18n.init();
	await ready;
};
