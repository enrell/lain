import { describe, expect, it } from 'vitest';
import { gridStep, rows, type Box } from './grid-nav';

// Three columns of 100 px with 20 px gaps; seven cards, so the last row holds one.
const grid: Box[] = Array.from({ length: 7 }, (_, i) => ({ left: (i % 3) * 120, top: Math.floor(i / 3) * 300, width: 100 }));

describe('grid navigation', () => {
	it('reads rows from where the cards sit, a lifted card included', () => {
		const lifted = grid.map((b, i) => (i === 1 ? { ...b, top: b.top - 4 } : b));
		expect(rows(lifted)).toEqual([[0, 1, 2], [3, 4, 5], [6]]);
	});

	it('moves along a row and wraps to the next or previous one', () => {
		expect(gridStep(grid, 0, 'ArrowRight')).toBe(1);
		expect(gridStep(grid, 2, 'ArrowRight')).toBe(3);
		expect(gridStep(grid, 3, 'ArrowLeft')).toBe(2);
		expect(gridStep(grid, 0, 'ArrowLeft')).toBeNull();
		expect(gridStep(grid, 6, 'ArrowRight')).toBeNull();
	});

	it('moves between rows to the nearest card, and up out of the first row', () => {
		expect(gridStep(grid, 1, 'ArrowDown')).toBe(4);
		expect(gridStep(grid, 5, 'ArrowDown')).toBe(6);
		expect(gridStep(grid, 6, 'ArrowUp')).toBe(3);
		expect(gridStep(grid, 1, 'ArrowUp')).toBe('out');
		expect(gridStep(grid, 6, 'ArrowDown')).toBeNull();
	});

	it('jumps to the ends and ignores other keys', () => {
		expect(gridStep(grid, 4, 'Home')).toBe(0);
		expect(gridStep(grid, 4, 'End')).toBe(6);
		expect(gridStep(grid, 4, 'Enter')).toBeNull();
		expect(gridStep(grid, 9, 'ArrowDown')).toBeNull();
	});
});
