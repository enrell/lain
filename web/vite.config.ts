import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

// Dev: Vite serves the app with HMR and proxies the gateway so the
// browser sees same-origin paths (no CORS, Range headers pass through).
// Prod: the Go process serves both the API and the embedded build.
const backend = process.env.LAIN_BACKEND ?? 'http://127.0.0.1:9360';

// LAIN_DEMO=1 builds the site's live demo (see demo/build.mjs).
const demo = process.env.LAIN_DEMO === '1';

// The demo's router takes hash routes; src/lib/demo/navigation.ts gives the
// client's absolute paths that form. It stands in for `$app/navigation` in
// every file of src/ but itself. The import is rewritten in the source text:
// SvelteKit's alias resolves `$app/*` natively, before any plugin is asked.
const wrapper = fileURLToPath(new URL('./src/lib/demo/navigation.ts', import.meta.url)).replace(/\\/g, '/');
const sources = fileURLToPath(new URL('./src/', import.meta.url)).replace(/\\/g, '/');
const demoNavigation = {
	name: 'lain-demo-navigation',
	enforce: 'pre' as const,
	transform(code: string, id: string) {
		const file = id.split('?')[0].replace(/\\/g, '/');
		if (!file.startsWith(sources) || file === wrapper || !code.includes('$app/navigation')) return null;
		return { code: code.replace(/(['"])\$app\/navigation\1/g, "'$lib/demo/navigation'"), map: null };
	}
};

export default defineConfig({
	plugins: [...(demo ? [demoNavigation] : []), tailwindcss(), sveltekit()],
	define: { 'import.meta.env.VITE_DEMO': JSON.stringify(demo) },
	server: {
		port: 5173,
		proxy: {
			'/api': { target: backend, changeOrigin: false },
			'/health': { target: backend, changeOrigin: false }
		}
	},
	test: {
		environment: 'node',
		include: ['src/**/*.test.ts']
	}
});
