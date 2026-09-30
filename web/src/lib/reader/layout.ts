/*
 * Pure reader logic: spread pairing, navigation and progress mapping.
 * The components only render what these functions decide, which keeps
 * the reading-direction rules testable without a browser.
 */

export type Direction = 'rtl' | 'ltr';

export interface ReaderPage {
	index: number;
	mime: string;
	size: number;
	/** 0 when the server could not read the header. */
	width: number;
	height: number;
}

export interface SpreadOptions {
	/** Show two pages side by side where they pair up. */
	dual: boolean;
	/** The first page (the cover) stands alone so later pairs line up. */
	coverAlone: boolean;
}

/** A wide page is already a printed spread and never pairs. */
export function isWide(page: ReaderPage): boolean {
	return page.width > 0 && page.height > 0 && page.width > page.height * 1.1;
}

/**
 * Group pages into what is on screen at once. Each spread lists page
 * indices in reading order; use visualOrder to lay them out.
 */
export function buildSpreads(pages: ReaderPage[], opts: SpreadOptions): number[][] {
	const out: number[][] = [];
	let i = 0;
	while (i < pages.length) {
		const alone =
			!opts.dual || isWide(pages[i]) || (opts.coverAlone && i === 0) || i === pages.length - 1;
		if (alone || isWide(pages[i + 1])) {
			out.push([i]);
			i += 1;
		} else {
			out.push([i, i + 1]);
			i += 2;
		}
	}
	return out;
}

/** Left-to-right screen order of a spread: manga reads right to left. */
export function visualOrder(spread: number[], direction: Direction): number[] {
	return direction === 'rtl' ? [...spread].reverse() : spread;
}

export function spreadContaining(spreads: number[][], page: number): number {
	const at = spreads.findIndex((s) => s.includes(page));
	return at < 0 ? 0 : at;
}

export type NavAction = 'next' | 'prev' | 'first' | 'last' | 'toggle-ui' | null;

/** A tap on the left or right third turns the page toward that side's direction. */
export function tapAction(x: number, width: number, direction: Direction): NavAction {
	if (width <= 0) return null;
	const f = x / width;
	if (f > 1 / 3 && f < 2 / 3) return 'toggle-ui';
	const left = f <= 1 / 3;
	if (direction === 'rtl') return left ? 'next' : 'prev';
	return left ? 'prev' : 'next';
}

export function keyAction(key: string, shift: boolean, direction: Direction): NavAction {
	switch (key) {
		case 'ArrowRight':
			return direction === 'rtl' ? 'prev' : 'next';
		case 'ArrowLeft':
			return direction === 'rtl' ? 'next' : 'prev';
		case 'PageDown':
		case 'ArrowDown':
			return 'next';
		case 'PageUp':
		case 'ArrowUp':
			return 'prev';
		case ' ':
			return shift ? 'prev' : 'next';
		case 'Home':
			return 'first';
		case 'End':
			return 'last';
		default:
			return null;
	}
}

/** Reading progress rides the shared progress record: position is the 1-based page. */
export function progressFor(page: number, total: number): {
	position_sec: number;
	duration_sec: number;
	completed: boolean;
} {
	const at = Math.min(Math.max(page, 0), Math.max(total - 1, 0));
	return { position_sec: at + 1, duration_sec: total, completed: total > 0 && at + 1 >= total };
}

/** Where to open: the saved page, or the start when finished or unread. */
export function startPage(
	progress: { position_sec: number; duration_sec: number; completed: boolean } | null,
	total: number
): number {
	if (!progress || progress.completed || progress.position_sec < 1) return 0;
	return Math.min(Math.max(Math.floor(progress.position_sec) - 1, 0), Math.max(total - 1, 0));
}

/** The direction a title reads in: a saved choice wins over the kind's default. */
export function resolveDirection(saved: string | null, fallback: Direction): Direction {
	return saved === 'rtl' || saved === 'ltr' ? saved : fallback;
}
