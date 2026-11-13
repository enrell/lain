import { describe, expect, it } from 'vitest';
import { PageTurns } from './turns';

function opened(page: number): PageTurns {
	const turns = new PageTurns();
	turns.open(page);
	return turns;
}

describe('PageTurns', () => {
	it('reports nothing for the page the reader opened on', () => {
		const turns = opened(7);
		expect(turns.turn('v1', 7, 8)).toBeNull();
		expect(turns.take()).toBeNull();
	});

	it('owes the newest turn until it is saved', () => {
		const turns = opened(0);
		turns.turn('v1', 1, 8);
		const snap = turns.turn('v1', 3, 8);
		expect(snap).toEqual({ position_sec: 4, duration_sec: 8, completed: false });
		expect(turns.take()).toEqual({ id: 'v1', snap });
		// Handed over once: a second hide event does not send it again.
		expect(turns.take()).toBeNull();
	});

	it('has nothing to send on hide once the newest turn landed', () => {
		const turns = opened(0);
		const snap = turns.turn('v1', 2, 8)!;
		turns.saved('v1', snap);
		expect(turns.take()).toBeNull();
	});

	it('keeps a newer turn owed when an older write lands', () => {
		// Page 4 is read while the write of page 2 is still in flight.
		const turns = opened(0);
		const older = turns.turn('v1', 1, 8)!;
		const newer = turns.turn('v1', 3, 8)!;
		turns.saved('v1', older);
		expect(turns.take()).toEqual({ id: 'v1', snap: newer });
	});

	it('counts the opened page once the reader has moved', () => {
		// Opened on page 5, turned to 6, came back to 5: 5 is where they are.
		const turns = opened(4);
		turns.turn('v1', 5, 8);
		expect(turns.turn('v1', 4, 8)).toEqual({ position_sec: 5, duration_sec: 8, completed: false });
		expect(turns.take()?.snap.position_sec).toBe(5);
	});

	it('marks the last page as finished', () => {
		const turns = opened(0);
		expect(turns.turn('v1', 7, 8)?.completed).toBe(true);
	});

	it('still owes the last volume\'s turn after the next one opens', () => {
		const turns = opened(0);
		const snap = turns.turn('v1', 7, 8)!;
		turns.open(0);
		expect(turns.turn('v2', 0, 8)).toBeNull();
		expect(turns.take()).toEqual({ id: 'v1', snap });
	});

	it('keeps the file a turn belongs to', () => {
		// A save for another file never clears this one's turn.
		const turns = opened(0);
		const snap = turns.turn('v1', 2, 8)!;
		turns.saved('v2', snap);
		expect(turns.take()?.id).toBe('v1');
	});
});
