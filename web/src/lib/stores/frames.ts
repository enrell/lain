import { api } from '$lib/api';
import type { CatalogItem, Enrichment } from '$lib/api/types';
import { session } from '$lib/auth/session.svelte';
import { isReadable } from '$lib/reader/kinds';
import type { SeriesGroup } from '$lib/utilities/grouping';
import { enrichmentCache, ensureEnrichments, libraryCache } from './media-cache.svelte';
import { frameDecided, frameOf, knownShape, measure, MEASURE_WAIT_MS, sampleArt, showsOwnStill, toldShape, type Shape, type StillOf } from './shapes.svelte';

/** The still a card shows for a file without artwork: its own cover or a frame. */
export function cardThumb(item: Pick<CatalogItem, 'id' | 'kind'>): string {
	return api.thumbnail.url(item.id, session.token, { width: isReadable(item.kind) ? 480 : 320 });
}

/** The picture a card shows first: poster, then cover, then the file's still (see Poster). */
export function cardPicture(item: Pick<CatalogItem, 'id' | 'kind'>, enrichment: Enrichment | null | undefined): string {
	return enrichment?.poster || enrichment?.cover || cardThumb(item);
}

/**
 * The file whose own still may teach its library's tally (see
 * `guessShape`): a public library's only, so what a private library holds
 * stays out of what this browser keeps. A library not listed yet counts
 * as private.
 */
export function stillOf(item: CatalogItem): StillOf | undefined {
	const library = libraryCache.get(item.library_id);
	return library && !library.private ? item : undefined;
}

/** Seconds into a video the server cuts its frame at when no time is asked for (D-269). */
const FRAME_AT = 10;

/**
 * The still for a 16:9 slot that stands for one video file (an episode's
 * row, a continue card): the file's own picture, unless the server says
 * that is an upright still (D-282), which beside a show's first episode is
 * the show's poster: then the file's frame, as its neighbours show. Cut
 * to the slot the poster lost its title; it was the show's, not the
 * episode's.
 */
export function wideStill(item: Pick<CatalogItem, 'id' | 'kind' | 'thumbnail'>, width: number): string {
	const upright = !isReadable(item.kind) && toldShape(item) === 'portrait';
	return api.thumbnail.url(item.id, session.token, upright ? { at: FRAME_AT, width } : { width });
}

/*
 * A set of titles (a grid, a rail) is drawn as soon as its titles are
 * known, in the frame `guessFrame` gives, and then checked once with
 * `settleFrame`: the frame changes only if the guess was wrong, and is
 * held after that, so it changes at most once and never as pictures
 * arrive one by one. A grid that waited for its pictures showed nothing
 * for seconds on a cold thumbnail cache. What the server tells of each
 * picture (D-282) makes the guess the answer, so the check measures only
 * the pictures nothing is known about.
 */

/** The frame a set's titles make by what is known now: overlays loaded, kinds, shapes measured. */
export function frameNow(groups: SeriesGroup[]): Shape {
	return frameOf(groups, (id) => enrichmentCache.get(id));
}

/**
 * The frame to draw a set in before any of its pictures has loaded: the
 * overlays of its first titles (one request when missing; they say which
 * pictures are posters), then `frameNow`. No picture is waited for.
 */
export async function guessFrame(groups: SeriesGroup[]): Promise<Shape> {
	await ensureEnrichments(sampleArt(groups).map((i) => i.id));
	return frameNow(groups);
}

/**
 * The frame a set's pictures make: its first pictures whose shape is not
 * known (`knownShape`) measured until they decide it (or all have
 * answered, or `MEASURE_WAIT_MS` is gone), then counted. The check a set
 * drawn in its guess gets once; a set whose pictures are all known
 * loads nothing. A still that came with its file is served at once while a
 * frame waits for ffmpeg on a cold cache, so the posters that prove a
 * guess wrong are the first to answer.
 */
export async function settleFrame(groups: SeriesGroup[]): Promise<Shape> {
	const art = sampleArt(groups);
	await ensureEnrichments(art.map((i) => i.id));
	const lookup = (id: string) => enrichmentCache.get(id);
	await measure(
		art
			.filter((i) => knownShape(i, lookup(i.id)) === undefined)
			.map((i) => {
				const enrichment = lookup(i.id);
				return { id: i.id, src: cardPicture(i, enrichment), still: showsOwnStill(i, enrichment) ? stillOf(i) : undefined };
			}),
		MEASURE_WAIT_MS,
		() => frameDecided(groups, lookup)
	);
	return frameNow(groups);
}
