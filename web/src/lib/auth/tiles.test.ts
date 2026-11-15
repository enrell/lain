import { describe, expect, it } from 'vitest';
import { tileCommand } from './tiles';

describe('account tile keys', () => {
	it('moves with the arrows and wraps', () => {
		expect(tileCommand('ArrowRight', 0, 3)).toEqual({ type: 'move', index: 1 });
		expect(tileCommand('ArrowDown', 2, 3)).toEqual({ type: 'move', index: 0 });
		expect(tileCommand('ArrowLeft', 0, 3)).toEqual({ type: 'move', index: 2 });
		expect(tileCommand('ArrowUp', 1, 3)).toEqual({ type: 'move', index: 0 });
	});

	it('picks with Enter, forgets with A and leaves the tiles on Esc', () => {
		expect(tileCommand('Enter', 1, 3)).toEqual({ type: 'pick' });
		expect(tileCommand('a', 1, 3)).toEqual({ type: 'forget' });
		expect(tileCommand('A', 1, 3)).toEqual({ type: 'forget' });
		expect(tileCommand('Escape', 1, 3)).toEqual({ type: 'form' });
	});

	it('ignores other keys and an empty row', () => {
		expect(tileCommand('b', 0, 3)).toBeNull();
		expect(tileCommand('Enter', 0, 0)).toBeNull();
		expect(tileCommand('Escape', 0, 0)).toBeNull();
	});
});
