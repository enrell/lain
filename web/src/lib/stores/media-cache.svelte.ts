import { SvelteMap } from 'svelte/reactivity';
import { api } from '$lib/api';
import type { CatalogItem, Enrichment, Library } from '$lib/api/types';

/*
 * Client-side lookups for data that pages reuse while navigating:
 * catalog items (detail, continue watching), enrichment overlays
 * (artwork) and the library list. Bounded by what the user actually
 * visits; enrichment lookups are batched so a grid costs one request.
 */

export const itemCache = new SvelteMap<string, CatalogItem>();
export const enrichmentCache = new SvelteMap<string, Enrichment | null>();
export const libraryCache = new SvelteMap<string, Library>();

const inflightItems = new Map<string, Promise<CatalogItem | undefined>>();
let inflightEnrich: Promise<void> | null = null;
let inflightLibraries: Promise<Library[]> | null = null;

function unique(ids: string[]): string[] {
	return [...new Set(ids.filter((id) => id !== ''))];
}

/** Fetches missing items (parallel) and returns them in input order. */
export async function ensureItems(ids: string[]): Promise<(CatalogItem | undefined)[]> {
	const wanted = unique(ids);
	const missing = wanted.filter((id) => !itemCache.has(id) && !inflightItems.has(id));
	await Promise.all(
		missing.map((id) => {
			const p = api.catalog
				.get(id)
				.then((item) => {
					itemCache.set(id, item);
					return item;
				})
				.catch(() => undefined)
				.finally(() => inflightItems.delete(id));
			inflightItems.set(id, p);
			return p;
		})
	);
	// Wait for requests started by a previous call too.
	await Promise.all(wanted.map((id) => inflightItems.get(id) ?? Promise.resolve()));
	return ids.map((id) => itemCache.get(id));
}

/** Loads overlays for ids whose state is unknown, in one batch. */
export async function ensureEnrichments(ids: string[]): Promise<void> {
	const missing = unique(ids).filter((id) => !enrichmentCache.has(id));
	if (missing.length > 0) {
		if (inflightEnrich) {
			await inflightEnrich;
			return ensureEnrichments(missing);
		}
		inflightEnrich = api.enrich
			.batch(missing)
			.then((items) => {
				for (const id of missing) enrichmentCache.set(id, null);
				for (const e of items) enrichmentCache.set(e.item_id, e);
			})
			.catch(() => {
				// Artwork is decoration: failure degrades to placeholders.
				for (const id of missing) enrichmentCache.set(id, null);
			})
			.finally(() => {
				inflightEnrich = null;
			});
		await inflightEnrich;
	}
}

/** Re-reads one overlay after an admin enrich/remove action. */
export async function refreshEnrichment(id: string): Promise<Enrichment | null> {
	const e = await api.enrich.get(id);
	enrichmentCache.set(id, e);
	return e;
}

export function applyEnrichment(e: Enrichment): void {
	enrichmentCache.set(e.item_id, e);
}

export function forgetEnrichment(id: string): void {
	enrichmentCache.set(id, null);
}

/** Libraries are a small list; cached with an explicit refresh. */
export async function ensureLibraries(force = false): Promise<Library[]> {
	if (force) {
		const libs = await api.libraries.list();
		libraryCache.clear();
		for (const lib of libs) libraryCache.set(lib.id, lib);
		return libs;
	}
	if (libraryCache.size > 0) return [...libraryCache.values()];
	if (inflightLibraries) return inflightLibraries;
	inflightLibraries = api.libraries
		.list()
		.then((libs) => {
			for (const lib of libs) libraryCache.set(lib.id, lib);
			return libs;
		})
		.finally(() => {
			inflightLibraries = null;
		});
	return inflightLibraries;
}

export function libraryName(id: string): string | undefined {
	return libraryCache.get(id)?.name;
}
