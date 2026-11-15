/**
 * Keys for the "who is watching" tiles (D-297). Arrows move, Enter
 * picks, A forgets the focused tile, Esc returns to the plain form.
 */
export type TileCommand = { type: 'move'; index: number } | { type: 'pick' } | { type: 'forget' } | { type: 'form' };

export function tileCommand(key: string, index: number, count: number): TileCommand | null {
	if (count <= 0) return null;
	const i = Math.min(Math.max(index, 0), count - 1);
	switch (key) {
		case 'ArrowRight':
		case 'ArrowDown':
			return { type: 'move', index: (i + 1) % count };
		case 'ArrowLeft':
		case 'ArrowUp':
			return { type: 'move', index: (i - 1 + count) % count };
		case 'Enter':
			return { type: 'pick' };
		case 'a':
		case 'A':
			return { type: 'forget' };
		case 'Escape':
			return { type: 'form' };
		default:
			return null;
	}
}
