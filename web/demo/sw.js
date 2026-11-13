// The live demo's server: a service worker that answers the client's API
// calls from a recorded snapshot (./api/index.json and ./api/files/), so
// the real client runs on a static host with nothing behind it.
//
// - Exact recorded GETs are served directly by normalized path + sorted query.
// - Unrecorded catalog, item playback, progress, tracking, batch, social,
//   acquire, and thumbnail/page variants synthesize valid answers from the
//   recorded catalog instead of 404ing.
// - Writes answer "ok" and change nothing.
const BASE = new URL('./', self.location).pathname;
const SNAPSHOT_VERSION = '__SNAPSHOT_VERSION__';
let indexPromise = null;
let storePromise = null;

self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));

function snapshot(force = false) {
	if (force || !indexPromise) {
		storePromise = null;
		indexPromise = fetch(BASE + 'api/index.json', { cache: 'no-store' }).then((r) => {
			if (!r.ok) throw new Error(`snapshot index ${r.status}`);
			return r.json();
		});
	}
	return indexPromise;
}

function keyOf(url) {
	const p = new URLSearchParams(url.search);
	p.delete('token');
	p.sort();
	const q = p.toString();
	return q ? `${url.pathname}?${q}` : url.pathname;
}

const json = (body, status = 200) =>
	new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

async function fetchSnapshotFile(file) {
	const res = await fetch(BASE + 'api/files/' + file, { cache: 'no-cache' });
	if (!res.ok) {
		const err = new Error(`snapshot asset missing: ${file} (${res.status})`);
		err.snapshotStale = res.status === 404;
		throw err;
	}
	return res;
}

async function recorded(entry) {
	const res = await fetchSnapshotFile(entry.file);
	return new Response(res.body, {
		status: 200,
		headers: { 'content-type': entry.type, 'cache-control': 'no-cache' }
	});
}

async function body(idx, key) {
	const e = idx[key];
	return e ? (await fetchSnapshotFile(e.file)).json() : null;
}

function compareItems(a, b) {
	const t = (a.title ?? '').localeCompare(b.title ?? '');
	if (t !== 0) return t;
	const s = (a.season ?? 0) - (b.season ?? 0);
	if (s !== 0) return s;
	const ep = (a.episode ?? 0) - (b.episode ?? 0);
	return ep !== 0 ? ep : (a.id ?? '').localeCompare(b.id ?? '');
}

async function buildStore(idx) {
	const [catAll, cont, usersList, listRes, feed, collections, friends, settings] = await Promise.all([
		body(idx, '/api/catalog?limit=500&offset=0&sort=title'),
		body(idx, '/api/me/continue'),
		body(idx, '/api/users'),
		body(idx, '/api/list'),
		body(idx, '/api/social/feed'),
		body(idx, '/api/social/collections'),
		body(idx, '/api/social/friends'),
		body(idx, '/api/social/me/settings')
	]);

	const items = (catAll?.items ?? []).slice().sort(compareItems);
	const byId = new Map();
	const byTitle = new Map();
	for (const item of items) {
		byId.set(item.id, item);
		let list = byTitle.get(item.title);
		if (!list) {
			list = [];
			byTitle.set(item.title, list);
		}
		list.push(item);
	}
	const firstByTitle = new Map();
	for (const [title, list] of byTitle) firstByTitle.set(title, list[0]);

	const keys = Object.keys(idx);
	const enrKeys = keys.filter((k) => k.startsWith('/api/enrichments?'));
	const durKeys = keys.filter((k) => k.startsWith('/api/durations?'));
	const workKeys = keys.filter((k) => k.startsWith('/api/social/work?item='));
	const pbKeys = keys.filter((k) => /^\/api\/items\/[^/]+\/playback(\?|$)/.test(k));
	const trkKeys = keys.filter((k) => /^\/api\/items\/[^/]+\/tracking$/.test(k));
	const progKeys = keys.filter((k) => /^\/api\/items\/[^/]+\/progress$/.test(k));

	const [enrPayloads, durPayloads, workPayloads, progPayloads] = await Promise.all([
		Promise.all(enrKeys.map((k) => body(idx, k))),
		Promise.all(durKeys.map((k) => body(idx, k))),
		Promise.all(workKeys.map((k) => body(idx, k))),
		Promise.all(progKeys.map((k) => body(idx, k)))
	]);

	const enrichmentById = new Map();
	for (const payload of enrPayloads) {
		for (const item of payload?.items ?? []) enrichmentById.set(item.item_id, item);
	}

	const durationById = new Map();
	for (const payload of durPayloads) {
		for (const item of payload?.items ?? []) durationById.set(item.item_id, item.duration_sec);
	}

	const progressById = new Map();
	for (const row of cont ?? []) progressById.set(row.item_id, row);
	for (const row of progPayloads) {
		if (row?.item_id) progressById.set(row.item_id, row);
	}

	const playbackKeyById = new Map();
	for (const k of pbKeys) playbackKeyById.set(k.split('/')[3], k);

	const trackingKeyById = new Map();
	for (const k of trkKeys) trackingKeyById.set(k.split('/')[3], k);

	const worksByItem = new Map();
	for (const payload of workPayloads) {
		const id = payload?.work?.item_id;
		if (id) worksByItem.set(id, payload);
	}

	const userMap = {};
	const userById = new Map();
	for (const u of usersList ?? []) {
		userById.set(u.id, u);
		userMap[u.id] = {
			id: u.id,
			username: u.username,
			display_name: u.profile?.display_name || (u.username ? u.username[0].toUpperCase() + u.username.slice(1) : undefined),
			avatar: u.profile?.avatar ?? {}
		};
	}

	return {
		items,
		byId,
		byTitle,
		firstByTitle,
		enrichmentById,
		durationById,
		progressById,
		playbackKeyById,
		trackingKeyById,
		worksByItem,
		usersList: usersList ?? [],
		userById,
		userMap,
		listEntries: listRes?.entries ?? [],
		feed: feed ?? { items: [], users: {} },
		collections: collections ?? { collections: [], users: {} },
		friends: friends ?? { friends: [], incoming: [], outgoing: [], blocked: [] },
		settings
	};
}

function storeFor(idx) {
	storePromise ??= buildStore(idx);
	return storePromise;
}

// Titles whose name holds every word of the query, from the widest catalog
// answer on record, in that answer's own shape.
async function search(idx, url) {
	const words = (url.searchParams.get('q') ?? '').toLowerCase().split(/\s+/).filter(Boolean);
	const all = await body(idx, '/api/catalog?group=title&limit=500');
	if (!all) return json({ items: [], total: 0 });
	const lib = url.searchParams.get('library_id');
	const pool = lib ? all.items.filter((i) => i.library_id === lib) : all.items;
	const items = words.length ? pool.filter((i) => words.every((w) => `${i.title} ${i.series ?? ''}`.toLowerCase().includes(w))) : [];
	const titles = new Set(items.map((i) => i.title)).size;
	return json({ ...all, items, total: items.length, titles });
}

function queryCatalog(store, url) {
	const lib = url.searchParams.get('library_id');
	const sort = url.searchParams.get('sort') ?? 'title';
	const group = url.searchParams.get('group');
	const limit = Math.max(1, Number.parseInt(url.searchParams.get('limit') ?? '50', 10) || 50);
	const offset = Math.max(0, Number.parseInt(url.searchParams.get('offset') ?? '0', 10) || 0);

	const filtered = lib ? store.items.filter((it) => it.library_id === lib) : store.items.slice();
	if (group === 'title') {
		const groups = [];
		const seen = new Map();
		for (const item of filtered) {
			let list = seen.get(item.title);
			if (!list) {
				list = [];
				seen.set(item.title, list);
				groups.push({ title: item.title, items: list, updatedAt: item.updated_at ?? 0 });
			}
			list.push(item);
			const g = groups.find((x) => x.title === item.title);
			if (g && (item.updated_at ?? 0) > g.updatedAt) g.updatedAt = item.updated_at ?? 0;
		}
		if (sort === 'recent') groups.sort((a, b) => b.updatedAt - a.updatedAt || a.title.localeCompare(b.title));
		const pageItems = groups.slice(offset, offset + limit).flatMap((g) => g.items);
		return json({ group: 'title', items: pageItems, limit, offset, total: groups.length });
	}
	if (sort === 'recent') {
		filtered.sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0) || compareItems(a, b));
	}
	return json({ items: filtered.slice(offset, offset + limit), limit, offset, total: filtered.length });
}

function batchLookup(map, keyName, url) {
	const ids = (url.searchParams.get('ids') ?? '').split(',').filter(Boolean);
	const items = [];
	for (const id of ids) {
		if (!map.has(id)) continue;
		const val = map.get(id);
		items.push(keyName === 'duration_sec' ? { item_id: id, duration_sec: val } : val);
	}
	return json({ items });
}

async function answerPlayback(idx, store, item) {
	const directKey = store.playbackKeyById.get(item.id);
	if (directKey) return recorded(idx[directKey]);
	const rep = store.firstByTitle.get(item.title);
	const repKey = rep ? store.playbackKeyById.get(rep.id) : null;
	if (repKey) {
		const plan = await body(idx, repKey);
		if (plan) {
			const dur = store.durationById.get(item.id) ?? plan.duration_sec;
			return json({ ...plan, asset: `asset:${item.id}`, ...(dur != null ? { duration_sec: dur } : {}) });
		}
	}
	return json({
		asset: `asset:${item.id}`,
		available: true,
		duration_sec: store.durationById.get(item.id) ?? 16.0,
		mode: 'direct',
		streams: [
			{ index: 0, type: 'video', codec: 'h264', default: true, width: 640, height: 360 },
			{ index: 1, type: 'audio', codec: 'aac', default: true, channels: 1 }
		]
	});
}

async function answerItemSubroute(idx, store, id, subroute) {
	const item = store.byId.get(id);
	if (!item) return json({ error: 'Item not found.' }, 404);
	switch (subroute) {
		case 'playback':
			return answerPlayback(idx, store, item);
		case 'progress':
			return json(
				store.progressById.get(id) ?? {
					completed: false,
					duration_sec: store.durationById.get(id) ?? 0.0,
					item_id: '',
					position_sec: 0.0,
					updated_at: 0,
					user_id: ''
				}
			);
		case 'tracking': {
			const ownKey = store.trackingKeyById.get(id);
			if (ownKey) return recorded(idx[ownKey]);
			const rep = store.firstByTitle.get(item.title);
			const repKey = rep ? store.trackingKeyById.get(rep.id) : null;
			return repKey ? recorded(idx[repKey]) : json({ tracks: [] });
		}
		case 'sidecars':
			return json({ sidecars: [] });
		case 'chapters':
			return json({ chapters: [] });
		case 'subtitles':
			return json({ subtitles: [] });
		case 'storyboard':
			return json({ interval_sec: 10, width: 160, height: 90, columns: 5, rows: 1, count: 0 });
		default:
			return null;
	}
}

function pickThumbnailKey(idx, id, url) {
	const prefix = `/api/items/${id}/thumbnail`;
	const candidates = Object.keys(idx).filter((k) => k === prefix || k.startsWith(prefix + '?'));
	if (!candidates.length) return null;
	const wantStill = url.searchParams.has('art') || url.searchParams.has('t');
	const wantWidth = Number.parseInt(url.searchParams.get('w') ?? '480', 10) || 480;
	const pool = candidates.filter((k) => {
		const isStill = /[?&](art|t)=/.test(k);
		return wantStill ? isStill : !isStill;
	});
	const usable = pool.length ? pool : null;
	if (!usable) return null;
	usable.sort((a, b) => {
		const wa = Number.parseInt(new URL('http://x' + a).searchParams.get('w') ?? '480', 10) || 480;
		const wb = Number.parseInt(new URL('http://x' + b).searchParams.get('w') ?? '480', 10) || 480;
		return Math.abs(wa - wantWidth) - Math.abs(wb - wantWidth);
	});
	return usable[0];
}

async function answerImageFallback(idx, url) {
	const m = url.pathname.match(/^\/api\/items\/([^/]+)\/(thumbnail|cover|pages?\/\d+|storyboard\/image)$/);
	if (m) {
		const [, id, kind] = m;
		if (kind === 'thumbnail') {
			const direct = pickThumbnailKey(idx, id, url);
			if (direct) return recorded(idx[direct]);
			const store = await storeFor(idx);
			const item = store.byId.get(id);
			const rep = item ? store.firstByTitle.get(item.title) : null;
			if (rep) {
				const repThumb = pickThumbnailKey(idx, rep.id, url);
				if (repThumb) return recorded(idx[repThumb]);
			}
		}
		const prefix = `/api/items/${id}/${kind}`;
		const near = Object.keys(idx).find((k) => k === prefix || k.startsWith(prefix + '?'));
		if (near) return recorded(idx[near]);
		if (kind.startsWith('page')) {
			const anyPage = Object.keys(idx).find((k) => k.startsWith(`/api/items/${id}/pages/0`));
			if (anyPage) return recorded(idx[anyPage]);
		}
		return null;
	}
	if (url.pathname === '/api/theme') {
		const near = Object.keys(idx).find((k) => k === '/api/theme' || k.startsWith('/api/theme?'));
		return near ? recorded(idx[near]) : null;
	}
	return null;
}

function buildRatingsForUser(store, userId) {
	const seen = new Set();
	const ratings = [];
	for (const w of store.worksByItem.values()) {
		for (const r of w.ratings ?? []) {
			if (r.user_id !== userId) continue;
			const key = `${r.work?.kind}:${r.work?.title}`;
			if (seen.has(key)) continue;
			seen.add(key);
			ratings.push(r);
		}
		if (w.mine && w.mine.user_id === userId) {
			const key = `${w.mine.work?.kind}:${w.mine.work?.title}`;
			if (!seen.has(key)) {
				seen.add(key);
				ratings.push(w.mine);
			}
		}
	}
	return ratings.sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
}

function answerSocial(store, url) {
	const path = url.pathname;
	if (path === '/api/social/work') {
		const itemId = url.searchParams.get('item');
		const kind = url.searchParams.get('kind') ?? 'episode';
		const title = url.searchParams.get('title') ?? '';
		const item = itemId ? store.byId.get(itemId) : store.firstByTitle.get(title);
		const rep = item ? store.firstByTitle.get(item.title) : null;
		const existing = (item && store.worksByItem.get(item.id)) || (rep && store.worksByItem.get(rep.id));
		if (existing) return json(existing);
		return json({
			average: 0.0,
			comments: [],
			mine: null,
			ratings: [],
			scored: 0,
			users: {},
			work: item
				? { item_id: item.id, kind: item.kind, library_id: item.library_id, title: item.title }
				: { kind, title }
		});
	}
	if (path === '/api/social/users') {
		const q = (url.searchParams.get('q') ?? '').trim().toLowerCase();
		const users = Object.values(store.userMap)
			.filter((u) => u.id !== 'user-admin')
			.filter((u) => !q || `${u.username} ${u.display_name ?? ''}`.toLowerCase().includes(q))
			.map((u) => ({
				relation: store.friends.friends?.some((f) => f.user.id === u.id) ? 'friend' : 'none',
				user: u
			}));
		return json({ users });
	}
	if (path === '/api/social/feed') {
		const before = Number.parseInt(url.searchParams.get('before') ?? '0', 10);
		const allItems = store.feed.items ?? [];
		const items = before ? allItems.filter((a) => a.at < before) : allItems;
		return json({ items, users: store.feed.users ?? store.userMap });
	}
	const colMatch = path.match(/^\/api\/social\/collections\/([^/]+)$/);
	if (colMatch) {
		const cid = decodeURIComponent(colMatch[1]);
		const col = (store.collections.collections ?? []).find((c) => c.id === cid);
		return col ? json({ collection: col, users: store.collections.users ?? store.userMap }) : json({ error: 'Collection not found.' }, 404);
	}
	const userMatch = path.match(/^\/api\/social\/users\/([^/]+)(?:\/(activity|ratings|collections))?$/);
	if (userMatch) {
		const uid = decodeURIComponent(userMatch[1]);
		const sub = userMatch[2];
		const user = store.userMap[uid];
		if (!user) return json({ error: 'User not found.' }, 404);
		if (!sub) {
			const brunoFavs = [
				{
					item_id: 'cb6d722e4a2e8c2e',
					kind: 'episode',
					library_id: 'lib-153529bd',
					title: store.byId.get('cb6d722e4a2e8c2e')?.title ?? 'Puella Magi Madoka Magica'
				},
				{ item_id: 'e2bf85f3b48199b6', kind: 'video', library_id: 'lib-3fc1dc28', title: 'Sintel' }
			];
			return json({
				user,
				relation: uid === 'user-admin' ? 'none' : 'friend',
				self: uid === 'user-admin',
				visible: { profile: true, activity: true, ratings: true },
				bio: uid === 'user-admin' ? 'Curating sci-fi shows, short films, and comics on lain.' : 'Weekend sci-fi and open-source animation fan.',
				favorites: uid === 'user-admin' ? (store.settings?.favorites ?? []) : brunoFavs
			});
		}
		if (sub === 'activity') {
			const before = Number.parseInt(url.searchParams.get('before') ?? '0', 10);
			const items = (store.feed.items ?? []).filter((a) => a.user_id === uid && (!before || a.at < before));
			return json({ items, users: store.userMap });
		}
		if (sub === 'ratings') {
			return json({ ratings: buildRatingsForUser(store, uid) });
		}
		if (sub === 'collections') {
			const collections = (store.collections.collections ?? []).filter((c) => c.owner_id === uid);
			return json({ collections, users: store.userMap });
		}
	}
	return null;
}

function answerList(store, url) {
	const typeFilter = url.searchParams.get('type');
	const statusFilter = url.searchParams.get('status');
	const entries = store.listEntries.filter((e) => {
		if (statusFilter && e.status !== statusFilter) return false;
		if (typeFilter === 'anime') return e.media_type === 'anime';
		if (typeFilter === 'manga') return e.media_type === 'manga' || e.media_type === 'comic';
		if (typeFilter && e.media_type !== typeFilter) return false;
		return true;
	});
	return json({ entries });
}

async function answer(request, url) {
	if (url.pathname === '/api/events') {
		// A stream that stays open and says nothing: the client's live updates.
		return new Response(new ReadableStream({ start() {} }), { headers: { 'content-type': 'text/event-stream' } });
	}
	if (request.method !== 'GET') return json({ status: 'ok', demo: true });
	const idx = await snapshot();
	const exact = idx[keyOf(url)];
	if (exact) return recorded(exact);

	if (url.pathname === '/api/search') return search(idx, url);

	const imgRes = await answerImageFallback(idx, url);
	if (imgRes) return imgRes;

	const store = await storeFor(idx);
	if (url.pathname === '/api/catalog') return queryCatalog(store, url);

	const catMatch = url.pathname.match(/^\/api\/catalog\/([^/]+)(\/episodes)?$/);
	if (catMatch) {
		const id = decodeURIComponent(catMatch[1]);
		const item = store.byId.get(id);
		if (!item) return json({ error: 'Item not found.' }, 404);
		return catMatch[2] ? json({ items: store.byTitle.get(item.title) ?? [item] }) : json(item);
	}

	const itemMatch = url.pathname.match(/^\/api\/items\/([^/]+)\/(playback|progress|tracking|sidecars|chapters|subtitles|storyboard)$/);
	if (itemMatch) {
		return answerItemSubroute(idx, store, decodeURIComponent(itemMatch[1]), itemMatch[2]);
	}

	if (url.pathname === '/api/enrichments') return batchLookup(store.enrichmentById, 'enrichment', url);
	if (url.pathname === '/api/durations') return batchLookup(store.durationById, 'duration_sec', url);
	if (url.pathname === '/api/list') return answerList(store, url);

	if (url.pathname.startsWith('/api/social/')) {
		const socialRes = answerSocial(store, url);
		if (socialRes) return socialRes;
	}

	if (url.pathname === '/api/acquire/blocklist') return json({ blocklist: [] });
	if (url.pathname === '/api/acquire/replaced') return json({ files: [] });
	if (url.pathname === '/api/acquire/search') return json({ candidates: [], failures: [] });
	if (url.pathname === '/api/acquire/wanted' && idx['/api/acquire/wanted']) return recorded(idx['/api/acquire/wanted']);
	if (url.pathname === '/api/playback/options') return json({ default_delivery: 'hls', deliveries: ['direct'], qualities: [], effects: [] });

	return json({ error: 'not-in-demo', message: 'The demo has no answer for this.' }, 404);
}

async function answerWithRetry(request, url) {
	try {
		return await answer(request, url);
	} catch (err) {
		if (!err?.snapshotStale) throw err;
		await snapshot(true);
		return await answer(request, url);
	}
}

self.addEventListener('fetch', (e) => {
	const url = new URL(e.request.url);
	if (url.origin !== self.location.origin) return;
	if (!url.pathname.startsWith('/api/') && url.pathname !== '/health') return;
	e.respondWith(answerWithRetry(e.request, url).catch(() => json({ error: 'demo-failed' }, 500)));
});
