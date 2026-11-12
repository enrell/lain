import { SvelteMap } from 'svelte/reactivity';
import type { CatalogItem, Enrichment } from '$lib/api/types';
import { isReadable } from '$lib/reader/kinds';
import { artItem } from '$lib/utilities/browse';
import type { SeriesGroup } from '$lib/utilities/grouping';

/**
 * The shape of each title's picture, learned when it loads. A set of
 * cards (a grid, a rail) shares one frame, the shape most of its
 * pictures have; a picture of the other shape is contained inside that
 * frame over a blurred copy of itself, so nothing is cropped or
 * stretched and one row never mixes a poster with a widescreen card.
 *
 * The metadata cannot say it alone: a video's still is a poster when the
 * file came with one beside it (the server serves that instead of a
 * frame). So a set's first pictures are measured before it is drawn
 * (`measure`, `settleFrame`), and the frame is then held: a grid that
 * voted with its pictures as they arrived changed shape under the eye.
 */
export type Shape = 'portrait' | 'landscape';

const measured = new SvelteMap<string, Shape>();

export function recordShape(id: string, width: number, height: number): void {
	if (width <= 0 || height <= 0) return;
	const shape: Shape = width > height * 1.15 ? 'landscape' : 'portrait';
	if (measured.get(id) !== shape) measured.set(id, shape);
}

/** The measured shape, or `guess` until the picture has loaded. */
export function shapeOf(id: string, guess: Shape): Shape {
	return measured.get(id) ?? guess;
}

/** Width over height of the frame each shape gets. */
export const RATIO: Record<Shape, number> = { portrait: 2 / 3, landscape: 16 / 9 };

/**
 * What a card's picture is before it loads, in the order a card picks
 * one (poster, then cover, then the file's own still or first page):
 * poster art stands upright; a cover is the provider's wide banner; a
 * reading file's first page stands upright and a video's still is wide.
 */
export function guessShape(item: Pick<CatalogItem, 'kind'>, enrichment: Enrichment | null | undefined): Shape {
	if (enrichment?.poster) return 'portrait';
	if (enrichment?.cover) return 'landscape';
	return isReadable(item.kind) ? 'portrait' : 'landscape';
}

/** The shape of the picture a title's card shows (see `artItem`). */
export function pictureShape(group: SeriesGroup, enrichment: (id: string) => Enrichment | null | undefined): Shape {
	const art = artItem(group);
	return shapeOf(art.id, guessShape(art, enrichment(art.id)));
}

/**
 * How many titles from the start of a set decide its frame: about the
 * first screen. Pictures further down load only when scrolled to, so
 * letting them vote would reshape the whole grid mid-scroll.
 */
export const SAMPLE = 24;

/** The frame a set of cards shares: the shape most of its first pictures have; portrait on a tie. */
export function frameFor(shapes: Shape[]): Shape {
	const sample = shapes.slice(0, SAMPLE);
	const wide = sample.filter((s) => s === 'landscape').length;
	return wide * 2 > sample.length ? 'landscape' : 'portrait';
}

/** `frameFor` over titles, from their pictures as measured so far. */
export function frameOf(groups: SeriesGroup[], enrichment: (id: string) => Enrichment | null | undefined): Shape {
	return frameFor(groups.slice(0, SAMPLE).map((g) => pictureShape(g, enrichment)));
}

/** The files whose pictures decide a set's frame. */
export function sampleArt(groups: SeriesGroup[]): CatalogItem[] {
	return groups.slice(0, SAMPLE).map(artItem);
}

/** How long a set waits for its first pictures before it is drawn with what is known. */
export const MEASURE_WAIT_MS = 2500;

/**
 * Loads pictures not measured yet and records their shapes; resolves when
 * all have answered or after `wait` ms. The browser keeps them, so the
 * cards that show them next load from its cache.
 */
export function measure(pictures: { id: string; src: string }[], wait = MEASURE_WAIT_MS): Promise<void> {
	const todo = pictures.filter((p) => p.src !== '' && !measured.has(p.id));
	if (todo.length === 0 || typeof Image === 'undefined') return Promise.resolve();
	const loads = todo.map(
		(p) =>
			new Promise<void>((done) => {
				const img = new Image();
				img.decoding = 'async';
				img.referrerPolicy = 'no-referrer';
				img.onload = () => {
					recordShape(p.id, img.naturalWidth, img.naturalHeight);
					done();
				};
				img.onerror = () => done();
				img.src = p.src;
			})
	);
	return Promise.race([Promise.all(loads).then(() => undefined), new Promise<void>((done) => setTimeout(done, wait))]);
}
