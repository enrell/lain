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
 * A set is drawn as soon as its titles are known, in the frame guessed
 * from what is known before a picture loads (`guessShape`, `guessFrame`);
 * its first pictures are then measured (`measure`, `settleFrame`) and the
 * frame corrected once if the guess was wrong, then held: a grid that
 * voted with its pictures as they arrived changed shape under the eye,
 * and one that waited for them showed nothing for seconds on a cold cache.
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

/**
 * Whether the pictures measured so far decide a set's frame, whatever the
 * others turn out to be: more than half of its first titles measured wide,
 * or too few left to make them so. A check need not wait for the rest.
 */
export function frameDecided(groups: SeriesGroup[]): boolean {
	const art = sampleArt(groups);
	let wide = 0;
	let open = 0;
	for (const item of art) {
		const shape = measured.get(item.id);
		if (shape === undefined) open += 1;
		else if (shape === 'landscape') wide += 1;
	}
	return wide * 2 > art.length || (wide + open) * 2 <= art.length;
}

/** How long a set's check waits for its first pictures before it settles on what is known. */
export const MEASURE_WAIT_MS = 2500;

/**
 * Loads pictures not measured yet and records their shapes; resolves when
 * all have answered, when `enough` says the answers so far suffice, or
 * after `wait` ms. The rest still load and are recorded, and the browser
 * keeps them, so the cards that show them next load from its cache.
 */
export function measure(
	pictures: { id: string; src: string }[],
	wait = MEASURE_WAIT_MS,
	enough: () => boolean = () => false
): Promise<void> {
	const todo = pictures.filter((p) => p.src !== '' && !measured.has(p.id));
	if (todo.length === 0 || typeof Image === 'undefined' || enough()) return Promise.resolve();
	return new Promise<void>((resolve) => {
		let left = todo.length;
		const timer = setTimeout(resolve, wait);
		const answered = () => {
			left -= 1;
			if (left === 0 || enough()) {
				clearTimeout(timer);
				resolve();
			}
		};
		for (const p of todo) {
			const img = new Image();
			img.decoding = 'async';
			img.referrerPolicy = 'no-referrer';
			img.onload = () => {
				recordShape(p.id, img.naturalWidth, img.naturalHeight);
				answered();
			};
			img.onerror = answered;
			img.src = p.src;
		}
	});
}
