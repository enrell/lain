import adapter from '@sveltejs/adapter-static';
import { vitePreprocess } from '@sveltejs/vite-plugin-svelte';

/** @type {import('@sveltejs/kit').Config} */
export default {
	preprocess: vitePreprocess(),
	kit: {
		// Static SPA: the Go server embeds the build output and serves
		// index.html for client-side routes. No Node SSR in production.
		adapter: adapter({ fallback: 'index.html', strict: false }),
		typescript: {
			config: (config) => {
				config.compilerOptions.strict = true;
				return config;
			}
		}
	}
};
