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

/** The file a still belongs to, as far as its library's tally goes (see `stillKey`). */
export type StillOf = Pick<CatalogItem, 'library_id' | 'season' | 'episode'>;

/**
 * Records a loaded picture's shape. `still` is given when the picture is
 * the file's own still (see `showsOwnStill`): it then also teaches what
 * that library's stills are.
 */
export function recordShape(id: string, width: number, height: number, still?: StillOf): void {
	if (width <= 0 || height <= 0) return;
	const shape: Shape = width > height * 1.15 ? 'landscape' : 'portrait';
	// Counted once a visit: a card drawn again loads the same picture again.
	if (still && !measured.has(id)) noteStill(stillKey(still), shape);
	if (measured.get(id) !== shape) measured.set(id, shape);
}

/*
 * What a library's files come with. A video's still is a poster when its
 * file came with one (the server serves that in place of a frame), and
 * nothing the client is given says which; but a library's files are
 * mostly alike: a films folder of posters, a clips folder of frames. So
 * the stills measured in each library are tallied, and the shape most of
 * them had is the guess for its stills not measured yet. The tallies are
 * kept in this browser, so a later visit guesses right before a single
 * picture has loaded; without them the guess is a frame, and the check
 * after drawing corrects it once.
 */
const STILLS_KEY = 'lain.frames.stills';
/** Past this many stills a tally is halved, so what a library holds now outweighs what it held. */
export const STILLS_CAP = 64;
/** Stills seen upright, then wide. */
export type Tally = [portrait: number, landscape: number];

const stills = readStills();

/**
 * A library's tally, its later episodes apart: a show's first file is
 * the one its card shows and often the one that came with the show's
 * poster, while the episodes after it carry frames. A film, a clip or a
 * show's first episode counts with the library; the rest count apart, a
 * later season's first episode too: no show's card shows it (a card shows
 * the first season's), so its frame, met in Continue watching or a
 * search, must not count against the posters the cards show.
 */
export function stillKey(item: StillOf): string {
	return item.season > 1 || item.episode > 1 ? `${item.library_id}/later` : item.library_id;
}

function readStills(): Map<string, Tally> {
	const out = new Map<string, Tally>();
	try {
		const saved: unknown = JSON.parse(localStorage.getItem(STILLS_KEY) ?? '{}');
		for (const [key, tally] of Object.entries(saved ?? {})) {
			if (Array.isArray(tally) && tally.length === 2 && tally.every((n) => typeof n === 'number' && n >= 0)) {
				out.set(key, [tally[0], tally[1]]);
			}
		}
	} catch {
		// A browser that keeps nothing guesses from the kinds alone.
	}
	return out;
}

let saving: ReturnType<typeof setTimeout> | undefined;
function saveStills(): void {
	clearTimeout(saving);
	saving = undefined;
	try {
		localStorage.setItem(STILLS_KEY, JSON.stringify(Object.fromEntries(stills)));
	} catch {
		// Lasts for this visit.
	}
}
function noteStill(key: string, shape: Shape): void {
	stills.set(key, tallyStill(stills.get(key), shape));
	// One write for a grid's worth of pictures, not one per picture, and
	// the last before the page goes (a reload within the second).
	if (saving !== undefined || typeof window === 'undefined') return;
	saving = setTimeout(saveStills, 1000);
	window.addEventListener('pagehide', saveStills, { once: true });
}

export function tallyStill(tally: Tally | undefined, shape: Shape): Tally {
	const [portrait, landscape] = tally ?? [0, 0];
	const next: Tally = shape === 'portrait' ? [portrait + 1, landscape] : [portrait, landscape + 1];
	return next[0] + next[1] > STILLS_CAP ? [Math.round(next[0] / 2), Math.round(next[1] / 2)] : next;
}

/** The shape most of a tally's stills had; a frame (wide) on a tie, and nothing before any. */
export function usualStill(tally: Tally | undefined): Shape | undefined {
	if (!tally || tally[0] + tally[1] === 0) return undefined;
	return tally[0] > tally[1] ? 'portrait' : 'landscape';
}

/** Whether a card shows the file's own still: a video with no overlay art. */
export function showsOwnStill(item: Pick<CatalogItem, 'kind'>, enrichment: Enrichment | null | undefined): boolean {
	return !enrichment?.poster && !enrichment?.cover && !isReadable(item.kind);
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
 * reading file's first page stands upright; a video's still is what its
 * library's stills have been in this browser, else a wide frame. The
 * library's type says no more than the kind does: a comics or manga
 * library holds reading files, and a films or series library does not say
 * whether its files came with posters.
 */
export function guessShape(item: Pick<CatalogItem, 'kind'> & StillOf, enrichment: Enrichment | null | undefined): Shape {
	if (enrichment?.poster) return 'portrait';
	if (enrichment?.cover) return 'landscape';
	if (isReadable(item.kind)) return 'portrait';
	return usualStill(stills.get(stillKey(item))) ?? 'landscape';
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
 * `still` is set on a file's own still (see `recordShape`).
 */
export function measure(
	pictures: { id: string; src: string; still?: StillOf }[],
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
				recordShape(p.id, img.naturalWidth, img.naturalHeight, p.still);
				answered();
			};
			img.onerror = answered;
			img.src = p.src;
		}
	});
}
