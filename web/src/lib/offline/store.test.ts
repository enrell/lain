import { describe, expect, it } from 'vitest';
import type { CatalogItem } from '$lib/api/types';
import { createOfflineStore, OfflineError, type Deps } from './store';
import { INDEX_KEY } from './routes';

function fakeCache() {
	const m = new Map<string, Response>();
	return {
		m,
		match: async (k: RequestInfo | URL) => m.get(String(k))?.clone(),
		put: async (k: RequestInfo | URL, r: Response) => void m.set(String(k), r),
		delete: async (k: RequestInfo | URL) => m.delete(String(k))
	};
}

const item = (over: Partial<CatalogItem> = {}): CatalogItem =>
	({ id: 'v1', kind: 'manga', title: 'Tiny Blade', season: 1, episode: 0, ...over }) as CatalogItem;

function deps(cache: ReturnType<typeof fakeCache>, over: Partial<Deps> = {}, failPage = -1): Deps {
	const view = { kind: 'manga', format: 'cbz', direction: 'rtl', pages: [0, 1, 2].map((i) => ({ index: i, mime: 'image/png', size: 100, width: 1, height: 1 })) };
	return {
		open: async () => cache,
		drop: async () => (cache.m.clear(), true),
		get: async (path) => {
			if (path.endsWith('/pages')) return new Response(JSON.stringify(view));
			const page = /\/pages\/(\d+)$/.exec(path);
			if (page) {
				if (Number(page[1]) === failPage) throw new Error('network down');
				return new Response(new Uint8Array(100));
			}
			return new Response('{}');
		},
		estimate: async () => ({ quota: 10 * 1024 ** 3, usage: 0 }),
		cap: () => 1024 ** 3,
		now: () => 1000,
		ensureWorker: async () => {},
		...over
	};
}

describe('offline store', () => {
	it('saves every read the reader needs and indexes it', async () => {
		const cache = fakeCache();
		const store = createOfflineStore(deps(cache));
		const seen: number[] = [];
		const saved = await store.save(item(), (d) => seen.push(d));
		expect(saved).toMatchObject({ item_id: 'v1', pages: 3, bytes: 300, saved_at: 1000 });
		expect(seen).toEqual([1, 2, 3]);
		for (const k of ['/api/items/v1/pages', '/api/items/v1/pages/2', '/api/catalog/v1', '/api/catalog/v1/episodes', '/api/items/v1/progress', '/api/me', '/api/setup/status']) {
			expect(cache.m.has(k), k).toBe(true);
		}
		expect(await store.isSaved('v1')).toBe(true);
		expect((await store.usage()).used).toBe(300);
	});

	it('refuses before fetching pages when the budget is short', async () => {
		const cache = fakeCache();
		let workers = 0;
		const store = createOfflineStore(deps(cache, { cap: () => 1000, ensureWorker: async () => void workers++ }));
		await expect(store.save(item())).rejects.toMatchObject({ code: 'quota' });
		expect(workers).toBe(0);
		expect([...cache.m.keys()]).toEqual([]);
	});

	it('respects the browser quota under a generous cap', async () => {
		const store = createOfflineStore(deps(fakeCache(), { cap: () => 0, estimate: async () => ({ quota: 1000, usage: 900 }) }));
		await expect(store.save(item())).rejects.toBeInstanceOf(OfflineError);
	});

	it('rolls back a half-saved item', async () => {
		const cache = fakeCache();
		const store = createOfflineStore(deps(cache, {}, 1));
		await expect(store.save(item())).rejects.toMatchObject({ code: 'failed' });
		// Session copies may stay (they are not the item); no item key does.
		expect([...cache.m.keys()].filter((k) => k.includes('/v1'))).toEqual([]);
	});

	it('removes one item and keeps the others', async () => {
		const cache = fakeCache();
		const store = createOfflineStore(deps(cache));
		await store.save(item());
		await store.save(item({ id: 'v2', season: 2 }));
		await store.remove('v1');
		expect((await store.list()).map((s) => s.item_id)).toEqual(['v2']);
		expect([...cache.m.keys()].some((k) => k.includes('/v1'))).toBe(false);
		expect(cache.m.has(INDEX_KEY)).toBe(true);
	});

	it('refuses video for now', async () => {
		const store = createOfflineStore(deps(fakeCache()));
		await expect(store.save(item({ kind: 'anime' }))).rejects.toMatchObject({ code: 'not-readable' });
	});

	it('saves a reflowable book: manifest, chapters and the resources they name', async () => {
		const cache = fakeCache();
		const manifest = { format: 'epub', layout: 'reflowable', title: 'Tiny Book', direction: 'ltr', pages: 1, chapters: [{ index: 0, chars: 10, linear: true }, { index: 1, chars: 5, linear: true }] };
		const store = createOfflineStore(
			deps(cache, {
				get: async (path) => {
					if (path.endsWith('/book')) return new Response(JSON.stringify(manifest));
					if (path.includes('/chapters/0')) return new Response('<img src="/api/items/b1/book/res/OEBPS/a.png"><a href="https://x.example/api/items/b1/book/res/no.png">');
					if (path.includes('/chapters/1')) return new Response('<p>end</p><img src="/api/items/b1/book/res/OEBPS/a.png">');
					if (path.includes('/book/res/')) return new Response(new Uint8Array(50));
					return new Response('{}');
				}
			})
		);
		const saved = await store.save(item({ id: 'b1', kind: 'book', size: 1000 }));
		expect(saved).toMatchObject({ item_id: 'b1', title: 'Tiny Book', pages: 1 });
		for (const k of ['/api/items/b1/book', '/api/items/b1/book/chapters/0', '/api/items/b1/book/chapters/1', '/api/items/b1/book/res/OEBPS/a.png']) {
			expect(cache.m.has(k), k).toBe(true);
		}
		expect([...cache.m.keys()].some((k) => k.includes('no.png'))).toBe(false);
		await store.remove('b1');
		expect(cache.m.has('/api/items/b1/book/res/OEBPS/a.png')).toBe(false);
		expect(await store.isSaved('b1')).toBe(false);
	});

	it('saves a PDF book at the offline width', async () => {
		const cache = fakeCache();
		const store = createOfflineStore(
			deps(cache, {
				get: async (path) => {
					if (path.endsWith('/book')) return new Response(JSON.stringify({ format: 'pdf', layout: 'pages', title: 'P', direction: 'ltr', pages: 2 }));
					if (/\/pages\/\d+\?w=1536$/.test(path)) return new Response(new Uint8Array(70));
					return new Response('{}');
				}
			})
		);
		const saved = await store.save(item({ id: 'p1', kind: 'book' }));
		expect(saved.bytes).toBe(140);
		expect(cache.m.has('/api/items/p1/pages/1?w=1536')).toBe(true);
		expect(cache.m.has('/api/items/p1/pages')).toBe(true);
	});
});
