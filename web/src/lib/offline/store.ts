/*
 * Saved-for-offline items in this browser (first cut: comics and manga).
 * Everything lives in one Cache Storage cache, OFFLINE_CACHE, keyed the
 * way the service worker looks requests up (routes.ts): the reader
 * view, every page image, the catalog item, its episode list and the
 * progress record, plus an index of what is saved. Saving checks the
 * byte budget first (budget.ts) and rolls back a half-saved item.
 */
import { fetchRaw } from '$lib/api/client';
import type { BookManifest, CatalogItem, ReaderView } from '$lib/api/types';
import { effectiveBudget, fits, loadCap, type Estimate } from './budget';
import { INDEX_KEY, OFFLINE_CACHE, OFFLINE_PDF_WIDTH, itemKeys } from './routes';

export interface SavedItem {
	item_id: string;
	title: string;
	kind: string;
	season: number;
	episode: number;
	pages: number;
	bytes: number;
	saved_at: number;
	/** Every cache key of a saved book (its chapters and resources). */
	keys?: string[];
}

export class OfflineError extends Error {
	constructor(
		readonly code: 'unsupported' | 'quota' | 'not-readable' | 'failed',
		message: string,
		readonly need = 0,
		readonly budget = 0
	) {
		super(message);
		this.name = 'OfflineError';
	}
}

type CacheLike = Pick<Cache, 'match' | 'put' | 'delete'>;

export interface Deps {
	open: () => Promise<CacheLike>;
	drop: () => Promise<boolean>;
	get: (path: string) => Promise<Response>;
	estimate: () => Promise<Estimate>;
	cap: () => number;
	now: () => number;
	/** Makes sure the service worker that answers offline is installed. */
	ensureWorker: () => Promise<void>;
}

function browserDeps(): Deps {
	return {
		open: () => caches.open(OFFLINE_CACHE),
		drop: () => caches.delete(OFFLINE_CACHE),
		get: (path) => fetchRaw(path),
		estimate: async () => (navigator.storage?.estimate ? navigator.storage.estimate() : {}),
		cap: () => loadCap(),
		now: () => Math.floor(Date.now() / 1000),
		ensureWorker: registerWorker
	};
}

/** Offline needs Cache Storage and a service worker (secure context). */
export function offlineSupported(): boolean {
	return typeof caches !== 'undefined' && typeof navigator !== 'undefined' && 'serviceWorker' in navigator;
}

/**
 * The worker is registered only once something is saved offline, so a
 * browser that never uses offline runs exactly as before. A persisted
 * storage grant keeps the browser from evicting saved items under
 * pressure (best effort; browsers may decline).
 */
export async function registerWorker(): Promise<void> {
	if (!('serviceWorker' in navigator)) throw new OfflineError('unsupported', 'Service workers are unavailable.');
	await navigator.serviceWorker.register('/service-worker.js', {
		type: import.meta.env.DEV ? 'module' : 'classic'
	});
	await navigator.storage?.persist?.().catch(() => false);
}

export function createOfflineStore(deps: Deps = browserDeps()) {
	async function readIndex(cache: CacheLike): Promise<SavedItem[]> {
		const res = await cache.match(INDEX_KEY);
		if (!res) return [];
		try {
			const list = (await res.json()) as SavedItem[];
			return Array.isArray(list) ? list : [];
		} catch {
			return [];
		}
	}

	async function writeIndex(cache: CacheLike, list: SavedItem[]): Promise<void> {
		await cache.put(INDEX_KEY, new Response(JSON.stringify(list), { headers: { 'Content-Type': 'application/json' } }));
	}

	async function list(): Promise<SavedItem[]> {
		return readIndex(await deps.open());
	}

	async function usage(): Promise<{ used: number; budget: number }> {
		const used = (await list()).reduce((n, s) => n + s.bytes, 0);
		return { used, budget: effectiveBudget(deps.cap(), await deps.estimate(), used) };
	}

	async function isSaved(itemId: string): Promise<boolean> {
		return (await list()).some((s) => s.item_id === itemId);
	}

	/**
	 * Saves one comic or manga file. onProgress reports (done, total)
	 * pages. Fails with OfflineError('quota') before fetching anything
	 * when the pages cannot fit.
	 */
	async function save(item: CatalogItem, onProgress?: (done: number, total: number) => void): Promise<SavedItem> {
		if (item.kind === 'book') return saveBook(item, onProgress);
		if (item.kind !== 'comic' && item.kind !== 'manga') {
			throw new OfflineError('not-readable', 'Only comics, manga and books can be saved offline.');
		}
		const cache = await deps.open();
		const id = encodeURIComponent(item.id);
		const viewRes = await deps.get(`/api/items/${id}/pages`);
		const view = (await viewRes.clone().json()) as ReaderView;
		const need = view.pages.reduce((n, p) => n + Math.max(p.size, 0), 0) + 64 * 1024;
		const index = await readIndex(cache);
		const others = index.filter((s) => s.item_id !== item.id);
		const ours = others.reduce((n, s) => n + s.bytes, 0);
		const budget = effectiveBudget(deps.cap(), await deps.estimate(), ours);
		if (!fits(need, ours, budget)) {
			throw new OfflineError('quota', 'Not enough offline space in this browser.', need, budget);
		}
		await deps.ensureWorker();
		const keys = itemKeys(item.id, view.pages.length);
		try {
			await cache.put(`/api/items/${id}/pages`, viewRes);
			for (const path of [`/api/catalog/${id}`, `/api/catalog/${id}/episodes`, `/api/items/${id}/progress`]) {
				await cache.put(path, await deps.get(path));
			}
			// The app boots with these two reads; seed them so a cold start
			// offline right after saving still knows who is signed in.
			for (const path of ['/api/setup/status', '/api/me']) {
				await cache.put(path, await deps.get(path));
			}
			let bytes = 0;
			for (let i = 0; i < view.pages.length; i++) {
				const res = await deps.get(`/api/items/${id}/pages/${i}`);
				const blob = await res.clone().blob();
				bytes += blob.size;
				await cache.put(`/api/items/${id}/pages/${i}`, res);
				onProgress?.(i + 1, view.pages.length);
			}
			const saved: SavedItem = {
				item_id: item.id,
				title: item.title,
				kind: item.kind,
				season: item.season,
				episode: item.episode,
				pages: view.pages.length,
				bytes,
				saved_at: deps.now()
			};
			await writeIndex(cache, [...others, saved]);
			return saved;
		} catch (err) {
			for (const k of keys) await cache.delete(k);
			if (err instanceof OfflineError) throw err;
			throw new OfflineError('failed', err instanceof Error ? err.message : 'Saving failed.');
		}
	}

	/**
	 * Saves a book: its manifest and either every chapter with the
	 * resources they name (EPUB, MOBI, AZW3) or every page rendered at
	 * OFFLINE_PDF_WIDTH (PDF). The file's size bounds the estimate.
	 */
	async function saveBook(item: CatalogItem, onProgress?: (done: number, total: number) => void): Promise<SavedItem> {
		const cache = await deps.open();
		const id = encodeURIComponent(item.id);
		const manifestRes = await deps.get(`/api/items/${id}/book`);
		const book = (await manifestRes.clone().json()) as BookManifest;
		const pdf = book.layout === 'pages';
		const need = (pdf ? book.pages * 300 * 1024 : Math.max(item.size, 0) * 2) + 64 * 1024;
		const index = await readIndex(cache);
		const others = index.filter((s) => s.item_id !== item.id);
		const ours = others.reduce((n, s) => n + s.bytes, 0);
		const budget = effectiveBudget(deps.cap(), await deps.estimate(), ours);
		if (!fits(need, ours, budget)) {
			throw new OfflineError('quota', 'Not enough offline space in this browser.', need, budget);
		}
		await deps.ensureWorker();
		const keys = [...itemKeys(item.id, 0), `/api/items/${id}/book`];
		let bytes = 0;
		const put = async (key: string, res: Response) => {
			bytes += (await res.clone().blob()).size;
			await cache.put(key, res);
			keys.push(key);
		};
		try {
			await cache.put(`/api/items/${id}/book`, manifestRes);
			for (const path of [`/api/catalog/${id}`, `/api/catalog/${id}/episodes`, `/api/items/${id}/progress`, '/api/setup/status', '/api/me']) {
				await cache.put(path, await deps.get(path));
			}
			if (pdf) {
				await cache.put(`/api/items/${id}/pages`, await deps.get(`/api/items/${id}/pages`));
				for (let i = 0; i < book.pages; i++) {
					const key = `/api/items/${id}/pages/${i}?w=${OFFLINE_PDF_WIDTH}`;
					await put(key, await deps.get(key));
					onProgress?.(i + 1, book.pages);
				}
			} else {
				const chapters = book.chapters ?? [];
				const resources = new Set<string>();
				const prefix = `/api/items/${item.id}/book/res/`;
				for (let i = 0; i < chapters.length; i++) {
					const key = `/api/items/${id}/book/chapters/${i}`;
					const res = await deps.get(key);
					const html = await res.clone().text();
					// Root-relative resource URLs only, as the server writes them.
					for (const m of html.matchAll(/["'(]\s*(\/api\/items\/[^/"']+\/book\/res\/[^"'()\s<>?]+)/g)) {
						if (m[1].startsWith(prefix)) resources.add(m[1]);
					}
					await put(key, res);
					onProgress?.(i + 1, chapters.length + resources.size);
				}
				let done = chapters.length;
				for (const r of resources) {
					await put(r, await deps.get(r));
					onProgress?.(++done, chapters.length + resources.size);
				}
			}
			const saved: SavedItem = {
				item_id: item.id,
				title: book.title || item.title,
				kind: item.kind,
				season: item.season,
				episode: item.episode,
				pages: book.pages,
				bytes,
				saved_at: deps.now(),
				keys
			};
			await writeIndex(cache, [...others, saved]);
			return saved;
		} catch (err) {
			for (const k of keys) await cache.delete(k);
			if (err instanceof OfflineError) throw err;
			throw new OfflineError('failed', err instanceof Error ? err.message : 'Saving failed.');
		}
	}

	async function remove(itemId: string): Promise<void> {
		const cache = await deps.open();
		const index = await readIndex(cache);
		const entry = index.find((s) => s.item_id === itemId);
		for (const k of entry?.keys ?? itemKeys(itemId, entry?.pages ?? 0)) await cache.delete(k);
		await writeIndex(cache, index.filter((s) => s.item_id !== itemId));
	}

	/** Drops every saved item (and the cached session copies). */
	async function clear(): Promise<void> {
		await deps.drop();
	}

	return { list, usage, isSaved, save, remove, clear };
}

export type OfflineStore = ReturnType<typeof createOfflineStore>;

let shared: OfflineStore | null = null;
export function offlineStore(): OfflineStore {
	return (shared ??= createOfflineStore());
}
