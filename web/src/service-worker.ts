/// <reference types="@sveltejs/kit" />
/// <reference no-default-lib="true"/>
/// <reference lib="esnext" />
/// <reference lib="webworker" />
/*
 * Offline worker (docs/slices/web-offline.md). Registered only after the
 * user saves something offline (lib/offline/store.ts), never on boot.
 *
 * Network-first everywhere it answers: online, every request goes to the
 * server exactly as without a worker; a cached copy is used only when the
 * network fails. It answers navigations (the SPA shell), the build's own
 * assets, the two session reads the app boots with, and the reads of
 * saved items (routes.ts). Streams, transcodes, writes and everything
 * else pass through untouched.
 */
import { build, files, version } from '$service-worker';
import { OFFLINE_CACHE, cacheKey, classify } from '$lib/offline/routes';

const sw = self as unknown as ServiceWorkerGlobalScope;
const SHELL_CACHE = `lain-shell-${version}`;
const SHELL = ['/', ...build, ...files];
const assets = new Set([...build, ...files]);

sw.addEventListener('install', (event) => {
	event.waitUntil(
		caches
			.open(SHELL_CACHE)
			.then((c) => c.addAll(SHELL))
			.then(() => sw.skipWaiting())
	);
});

sw.addEventListener('activate', (event) => {
	event.waitUntil(
		caches
			.keys()
			.then((keys) => Promise.all(keys.filter((k) => k.startsWith('lain-shell-') && k !== SHELL_CACHE).map((k) => caches.delete(k))))
			.then(() => sw.clients.claim())
	);
});

async function networkFirst(request: Request, cacheName: string, key: string, refresh: 'always' | 'if-present'): Promise<Response> {
	const cache = await caches.open(cacheName);
	try {
		const res = await fetch(request);
		if (res.ok && (refresh === 'always' || (await cache.match(key)))) {
			await cache.put(key, res.clone());
		}
		return res;
	} catch (err) {
		const hit = await cache.match(key);
		if (hit) return hit;
		throw err;
	}
}

sw.addEventListener('fetch', (event) => {
	const req = event.request;
	const url = new URL(req.url);
	const route = classify(url, req.method, req.mode, sw.location.origin, assets);
	switch (route) {
		case 'navigate':
			// Every client route is the same SPA shell.
			event.respondWith(networkFirst(req, SHELL_CACHE, '/', 'always'));
			return;
		case 'asset':
			event.respondWith(
				caches.open(SHELL_CACHE).then(async (c) => (await c.match(url.pathname)) ?? fetch(req))
			);
			return;
		case 'session':
			// Kept fresh so a cold start offline still knows who is signed in.
			event.respondWith(networkFirst(req, OFFLINE_CACHE, cacheKey(url), 'always'));
			return;
		case 'saved':
			// Only items the user saved are cached; others are just fetched.
			event.respondWith(networkFirst(req, OFFLINE_CACHE, cacheKey(url), 'if-present'));
			return;
	}
});
