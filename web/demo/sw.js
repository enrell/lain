// The live demo's server: a service worker that answers the client's API
// calls from a recorded snapshot (./api/index.json and ./api/files/), so
// the real client runs on a static host with nothing behind it.
//
// - A GET is looked up by its path and query (without the session token).
// - A search that was not recorded is answered from the recorded catalog.
// - A picture asked for at another size gets the recorded one.
// - Anything that writes answers "ok" and changes nothing.
const BASE = new URL('./', self.location).pathname;
let index = null;

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

function snapshot() {
	index ??= fetch(BASE + 'api/index.json', { cache: 'no-cache' }).then((r) => r.json());
	return index;
}

function keyOf(url) {
	const p = new URLSearchParams(url.search);
	p.delete('token');
	p.sort();
	const q = p.toString();
	return q ? `${url.pathname}?${q}` : url.pathname;
}

const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

async function recorded(entry) {
	const res = await fetch(BASE + 'api/files/' + entry.file);
	return new Response(res.body, { status: 200, headers: { 'content-type': entry.type, 'cache-control': 'max-age=3600' } });
}

async function body(idx, key) {
	const e = idx[key];
	return e ? (await fetch(BASE + 'api/files/' + e.file)).json() : null;
}

// Titles whose name holds every word of the query, from the widest catalog
// answer on record, in that answer's own shape.
async function search(idx, url) {
	const words = (url.searchParams.get('q') ?? '').toLowerCase().split(/\s+/).filter(Boolean);
	const all = await body(idx, '/api/catalog?group=title&limit=500');
	if (!all) return json({ items: [], total: 0 });
	const items = words.length ? all.items.filter((i) => words.every((w) => `${i.title} ${i.series ?? ''}`.toLowerCase().includes(w))) : [];
	const titles = new Set(items.map((i) => i.title)).size;
	return json({ ...all, items, total: items.length, titles });
}

async function answer(request, url) {
	if (url.pathname === '/api/events') {
		// A stream that stays open and says nothing: the client's live updates.
		return new Response(new ReadableStream({ start() {} }), { headers: { 'content-type': 'text/event-stream' } });
	}
	if (request.method !== 'GET') return json({ status: 'ok', demo: true });
	const idx = await snapshot();
	const hit = idx[keyOf(url)];
	if (hit) return recorded(hit);
	if (url.pathname === '/api/search') return search(idx, url);
	if (/\/(thumbnail|cover|page\/\d+|storyboard\/image)$/.test(url.pathname) || url.pathname === '/api/theme') {
		const near = Object.keys(idx).find((k) => k === url.pathname || k.startsWith(url.pathname + '?'));
		if (near) return recorded(idx[near]);
	}
	return json({ error: 'not-in-demo', message: 'The demo has no answer for this.' }, 404);
}

self.addEventListener('fetch', (e) => {
	const url = new URL(e.request.url);
	if (url.origin !== self.location.origin) return;
	if (!url.pathname.startsWith('/api/') && url.pathname !== '/health') return;
	e.respondWith(answer(e.request, url).catch(() => json({ error: 'demo-failed' }, 500)));
});
