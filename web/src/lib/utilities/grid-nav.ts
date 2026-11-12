/*
 * Arrow keys across a grid of cards, decided from where the cards sit on
 * screen: a grid of `auto-fill` columns has as many columns as the window
 * allows, so a row is "the cards at one height", not a fixed count.
 */

export interface Box {
	left: number;
	top: number;
	width: number;
}

/** Cards whose tops are this close sit on one row (a hover lift moves one by a few pixels). */
const ROW_SLACK = 8;

/** The rows of a grid, each a list of indexes from left to right. */
export function rows(boxes: Box[]): number[][] {
	const out: { top: number; items: number[] }[] = [];
	boxes.forEach((box, i) => {
		const row = out.find((r) => Math.abs(r.top - box.top) <= ROW_SLACK);
		if (row) row.items.push(i);
		else out.push({ top: box.top, items: [i] });
	});
	out.sort((a, b) => a.top - b.top);
	for (const row of out) row.items.sort((a, b) => boxes[a].left - boxes[b].left);
	return out.map((r) => r.items);
}

/**
 * Where a key moves from card `from`: another card's index, `'out'` when
 * an up arrow leaves the first row (back to the field above the grid), or
 * `null` when the key does nothing here.
 */
export function gridStep(boxes: Box[], from: number, key: string): number | 'out' | null {
	if (from < 0 || from >= boxes.length) return null;
	if (key === 'Home') return 0;
	if (key === 'End') return boxes.length - 1;
	const grid = rows(boxes);
	const r = grid.findIndex((row) => row.includes(from));
	const c = grid[r].indexOf(from);
	if (key === 'ArrowLeft') {
		if (c > 0) return grid[r][c - 1];
		return r > 0 ? grid[r - 1][grid[r - 1].length - 1] : null;
	}
	if (key === 'ArrowRight') {
		if (c < grid[r].length - 1) return grid[r][c + 1];
		return r < grid.length - 1 ? grid[r + 1][0] : null;
	}
	if (key !== 'ArrowUp' && key !== 'ArrowDown') return null;
	const next = r + (key === 'ArrowUp' ? -1 : 1);
	if (next < 0) return 'out';
	if (next >= grid.length) return null;
	// The card under (or over) this one: the nearest centre in that row.
	const centre = boxes[from].left + boxes[from].width / 2;
	let best = grid[next][0];
	for (const i of grid[next]) {
		const d = Math.abs(boxes[i].left + boxes[i].width / 2 - centre);
		if (d < Math.abs(boxes[best].left + boxes[best].width / 2 - centre)) best = i;
	}
	return best;
}

/**
 * The keydown handler for a grid whose children are its cards (links):
 * the arrows, Home and End move between them, and an up arrow out of the
 * first row hands focus to `above` (the field the results came from).
 * Enter is the link's own.
 */
export function gridKeys(e: KeyboardEvent, above?: () => HTMLElement | null | undefined): void {
	if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
	const grid = e.currentTarget as HTMLElement;
	const cards = [...grid.children] as HTMLElement[];
	const from = cards.findIndex((card) => card === e.target || card.contains(e.target as Node));
	if (from < 0) return;
	const step = gridStep(
		cards.map((card) => {
			const r = card.getBoundingClientRect();
			return { left: r.left, top: r.top, width: r.width };
		}),
		from,
		e.key
	);
	if (step === null) return;
	const target = step === 'out' ? above?.() : cards[step];
	if (!target) return;
	e.preventDefault();
	// A card's link is the card itself or its first link.
	const focusable = target.matches('a, button') ? target : (target.querySelector<HTMLElement>('a, button') ?? target);
	if (step === 'out') {
		focusable.focus();
		return;
	}
	// Scrolled by hand so the card's scroll margin (the sticky bars above
	// the grid) is honoured; focus alone could leave it under them.
	focusable.focus({ preventScroll: true });
	focusable.scrollIntoView({ block: 'nearest' });
}
