import type { TranscodeQuality } from '$lib/api/types';

/** A box side: 0 or absent is unbounded. */
const side = (n: number | undefined): number => (n && n > 0 ? n : Infinity);

/** Whether box `a` fits inside box `b` on both sides. */
const within = (a: TranscodeQuality, b: TranscodeQuality): boolean =>
	side(a.max_width) <= side(b.max_width) && side(a.max_height) <= side(b.max_height);

const boxed = (q: TranscodeQuality): boolean => side(q.max_width) < Infinity || side(q.max_height) < Infinity;

/**
 * The quality rungs worth offering for a `width`x`height` source.
 *
 * The server never enlarges a picture: a rung caps the width and height
 * at its box. A rung the source does not fit in shrinks it, so it is
 * offered. Of the rungs the source fits in, only the smallest are: that
 * is the file's own size (1080p for a 1920x800 film, whose height alone
 * is under 1080), and a larger box would leave the same picture. Both
 * sides count, not the height alone, or a widescreen film loses its own
 * rung. Rungs with the same box differ in bitrate, so they stay together.
 *
 * A rung without a box (a bitrate cap only) is always offered, and an
 * unknown source size offers the whole ladder.
 */
export function offeredQualities(
	qualities: TranscodeQuality[],
	width: number | undefined,
	height: number | undefined
): TranscodeQuality[] {
	if (!width || !height) return qualities;
	const source: TranscodeQuality = { name: '', max_width: width, max_height: height };
	const fits = qualities.filter((q) => boxed(q) && within(source, q));
	return qualities.filter(
		(q) => !fits.includes(q) || !fits.some((other) => within(other, q) && !within(q, other))
	);
}
