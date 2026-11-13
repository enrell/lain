import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

// LAIN_DEMO=1 builds the client as the site's live demo: it runs under a
// sub-path of a static host with no server behind it (a service worker
// answers the API from a recorded snapshot), so routes live in the URL's
// hash and every asset is relative to the base.
const demo = process.env.LAIN_DEMO === '1';
const demoBase = process.env.LAIN_DEMO_BASE ?? '/lain/demo';

/** @type {import('@sveltejs/kit').Config} */
export default {
	preprocess: vitePreprocess(),
	kit: {
		// Static SPA: the Go server embeds the build output and serves
		// index.html for client-side routes. No Node SSR in production.
		adapter: adapter({ fallback: 'index.html', strict: false }),
		// The offline worker (src/service-worker.ts) is registered by the
		// app only once something is saved offline, never on boot.
		serviceWorker: { register: false },
		...(demo ? { paths: { base: demoBase }, router: { type: 'hash' } } : {}),
		typescript: {
			config: (config) => {
				config.compilerOptions.strict = true;
				return config;
			}
		}
	}
};
