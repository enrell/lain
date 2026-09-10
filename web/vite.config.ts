import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vitest/config';

// Dev: Vite serves the app with HMR and proxies the gateway so the
// browser sees same-origin paths (no CORS, Range headers pass through).
// Prod: the Go process serves both the API and the embedded build.
const backend = process.env.LAIN_BACKEND ?? 'http://127.0.0.1:9360';

export default defineConfig({
	plugins: [tailwindcss(), sveltekit()],
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
