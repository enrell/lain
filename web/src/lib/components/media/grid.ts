/*
 * The sizes of a set of cards that share one frame (see `guessFrame`).
 * A tile is about the size of Home's rail cards for its shape, so a
 * poster grid and a widescreen grid carry the same weight wherever titles
 * are listed: a library, the favourites, search results, a rail.
 */
import type { Shape } from '$lib/stores/shapes.svelte';

/*
 * Equal columns of one frame, as many as fit: a poster 10-11rem wide, a
 * widescreen card 20-22rem; a phone takes two posters or one wide card a
 * row, the small tiles three or two. Empty tracks are kept (auto-fill),
 * so three favourites do not swell to the page's width.
 */
const FRAMED: Record<Shape, string> = {
	portrait: 'grid grid-cols-2 items-start gap-x-3.5 gap-y-6 sm:grid-cols-[repeat(auto-fill,minmax(9.5rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(10rem,1fr))]',
	landscape: 'grid grid-cols-1 items-start gap-x-3.5 gap-y-6 sm:grid-cols-[repeat(auto-fill,minmax(16rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(19.5rem,1fr))]'
};
const FRAMED_DENSE: Record<Shape, string> = {
	portrait: 'grid grid-cols-3 items-start gap-x-3 gap-y-5 sm:grid-cols-[repeat(auto-fill,minmax(7rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(7.5rem,1fr))]',
	landscape: 'grid grid-cols-2 items-start gap-x-3 gap-y-5 sm:grid-cols-[repeat(auto-fill,minmax(12rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(14rem,1fr))]'
};

export function framedGrid(frame: Shape, dense = false): string {
	return (dense ? FRAMED_DENSE : FRAMED)[frame];
}

/*
 * A rail of one frame sets its cards' width (`--card-w`, for a wrapper
 * of `w-[var(--card-w)]`) and the frame's ratio (`--r`) by its shape, as
 * Home's rails are sized: posters 10-12rem wide, widescreen cards
 * 20-24rem. One height for every rail made a poster a third of a
 * widescreen card's area.
 */
export const RAIL: Record<Shape, string> = {
	portrait: '[--card-w:min(40vw,12rem)] [--r:0.6667] sm:[--card-w:10rem] lg:[--card-w:11rem] xl:[--card-w:12rem]',
	landscape: '[--card-w:min(78vw,24rem)] [--r:1.7778] sm:[--card-w:20rem] lg:[--card-w:22rem] xl:[--card-w:24rem]'
};
