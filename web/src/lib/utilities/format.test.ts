import { describe, expect, it } from 'vitest';
import { elapsedSince } from './format';

describe('elapsedSince', () => {
	it('counts seconds from the server timestamp', () => {
		expect(elapsedSince(1000, 1_012_500)).toBeCloseTo(12.5);
	});

	it('never goes negative when the server clock leads the browser', () => {
		expect(elapsedSince(2000, 1_000_000)).toBe(0);
	});

	it('returns zero without a timestamp', () => {
		expect(elapsedSince(0, 1_000_000)).toBe(0);
		expect(elapsedSince(undefined, 1_000_000)).toBe(0);
		expect(elapsedSince(null, 1_000_000)).toBe(0);
	});
});
