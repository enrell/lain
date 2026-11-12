import { api } from '$lib/api';
import type { CatalogItem, Enrichment } from '$lib/api/types';
import { session } from '$lib/auth/session.svelte';
import { isReadable } from '$lib/reader/kinds';
import type { SeriesGroup } from '$lib/utilities/grouping';
import { enrichmentCache, ensureEnrichments } from './media-cache.svelte';
import { frameOf, measure, sampleArt, type Shape } from './shapes.svelte';

/** The still a card shows for a file without artwork: its own cover or a frame. */
export function cardThumb(item: Pick<CatalogItem, 'id' | 'kind'>): string {
	return api.thumbnail.url(item.id, session.token, { width: isReadable(item.kind) ? 480 : 320 });
}

/** The picture a card shows first: poster, then cover, then the file's still (see Poster). */
export function cardPicture(item: Pick<CatalogItem, 'id' | 'kind'>, enrichment: Enrichment | null | undefined): string {
	return enrichment?.poster || enrichment?.cover || cardThumb(item);
}

/**
 * Readies a set of titles to be drawn in one frame: the overlays of its
 * first titles, then their pictures measured, then the frame they make.
 * A grid or rail draws once this answers and keeps the frame, so it never
 * opens in one shape and turns to the other as pictures arrive.
 */
export async function settleFrame(groups: SeriesGroup[]): Promise<Shape> {
	const art = sampleArt(groups);
	await ensureEnrichments(art.map((i) => i.id));
	await measure(art.map((i) => ({ id: i.id, src: cardPicture(i, enrichmentCache.get(i.id)) })));
	return frameOf(groups, (id) => enrichmentCache.get(id));
}
