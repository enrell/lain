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

// The demo's router takes hash routes; src/lib/demo/{navigation,state,kit}.ts
// adapt SvelteKit's client routing APIs so components work identically with
// canonical paths and queries. SvelteKit resolves `$app/*` natively before any
// plugin unless rewritten in source text.
const demoRoot = fileURLToPath(new URL('./src/lib/demo/', import.meta.url)).replace(/\\/g, '/');
const sources = fileURLToPath(new URL('./src/', import.meta.url)).replace(/\\/g, '/');
const demoReplacements: Record<string, string> = {
	'$app/navigation': '$lib/demo/navigation',
	'$app/state': '$lib/demo/state',
	'@sveltejs/kit': '$lib/demo/kit'
};
const demoImports = {
	name: 'lain-demo-imports',
	enforce: 'pre' as const,
	transform(code: string, id: string) {
		const file = id.split('?')[0].replace(/\\/g, '/');
		if (!file.startsWith(sources) || file.startsWith(demoRoot)) return null;
		let changed = false;
		const next = code.replace(
			/(\b(?:from|import\s*\()\s*)(['"])(\$app\/(?:navigation|state)|@sveltejs\/kit)\2/g,
			(_, prefix: string, _q: string, spec: string) => {
				changed = true;
				return `${prefix}'${demoReplacements[spec]}'`;
			}
		);
		return changed ? { code: next, map: null } : null;
	}
};

export default defineConfig({
	plugins: [...(demo ? [demoImports] : []), tailwindcss(), sveltekit()],
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
