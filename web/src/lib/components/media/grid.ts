/*
 * The sizes of a set of cards that share one frame (see `guessFrame`).
 * Each shape has one card size wherever titles are listed (Home's rails,
 * a library's shelves, "More in", a library, the favourites, search
 * results), so a poster or a widescreen card weighs the same on every
 * page: a rail sets it (RAIL), and a grid fills its width with columns as
 * close to it as the width allows (FRAMED).
 *
 * The sizes, by breakpoint: a poster 10rem (sm), 10.5rem (lg), 12.5rem
 * (xl); a widescreen card 20rem (sm, lg), 22rem (xl). They were picked so
 * that both kinds of grid, the library's beside its filter column and the
 * full-width ones (favourites, search), land within about 5% of them at
 * 1440 and 1920 px and within 10% between: with the old 12rem posters a
 * library grid at 1440 drew 169 px posters beside 192 px rails.
 */
import type { Shape } from '$lib/stores/shapes.svelte';

/*
 * Equal columns of one frame, as many as fit: a column is at least 90% of
 * the card size, so the count lands on the one nearest to it and the
 * cards stretch only by what is left over. The gaps are the rails'. A
 * phone takes two posters or one wide card a row. Empty tracks are kept
 * (auto-fill), so three favourites do not swell to the page's width.
 */
const FRAMED: Record<Shape, string> = {
	portrait:
		'grid grid-cols-2 items-start gap-x-3.5 gap-y-6 md:gap-x-4 lg:gap-x-5 sm:grid-cols-[repeat(auto-fill,minmax(9rem,1fr))] lg:grid-cols-[repeat(auto-fill,minmax(9.45rem,1fr))] xl:grid-cols-[repeat(auto-fill,minmax(11.25rem,1fr))]',
	landscape:
		'grid grid-cols-1 items-start gap-x-3.5 gap-y-6 md:gap-x-4 lg:gap-x-5 sm:grid-cols-[repeat(auto-fill,minmax(18rem,1fr))] xl:grid-cols-[repeat(auto-fill,minmax(19.8rem,1fr))]'
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
 * of `w-[var(--card-w)]`), the frame's ratio (`--r`) and the gap, the
 * sizes above. One height for every rail made a poster a third of a
 * widescreen card's area.
 */
export const RAIL: Record<Shape, string> = {
	portrait: '[--card-w:min(40vw,12rem)] [--r:0.6667] sm:[--card-w:10rem] lg:[--card-w:10.5rem] xl:[--card-w:12.5rem] lg:gap-5',
	landscape: '[--card-w:min(78vw,24rem)] [--r:1.7778] sm:[--card-w:20rem] xl:[--card-w:22rem] lg:gap-5'
};
