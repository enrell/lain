// The demo build's stand-in for `@sveltejs/kit` (see vite.config.ts).
// `redirect(status, '/settings/profile')` under SvelteKit's hash router
// otherwise resolves against `current.url` as an origin-root path and triggers
// a full-page 404 outside the demo mount path.

import { redirect as kitRedirect } from '@sveltejs/kit';
import { hashed } from './url';

export * from '@sveltejs/kit';

export function redirect(status: Parameters<typeof kitRedirect>[0], location: string | URL): never {
	return kitRedirect(status, hashed(location));
}
