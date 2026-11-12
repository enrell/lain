import { describe, expect, it } from 'vitest';
import { edgesOf } from './scroll-edges';

describe('edgesOf', () => {
	it('hides more after the end at rest, and nothing before it', () => {
		expect(edgesOf(0, 1000, 400)).toEqual({ start: false, end: true });
	});
	it('hides more on both sides in the middle', () => {
		expect(edgesOf(300, 1000, 400)).toEqual({ start: true, end: true });
	});
	it('hides nothing after the end once it is reached, give or take a pixel', () => {
		expect(edgesOf(600, 1000, 400)).toEqual({ start: true, end: false });
		expect(edgesOf(599.5, 1000, 400)).toEqual({ start: true, end: false });
	});
	it('hides nothing when the row fits', () => {
		expect(edgesOf(0, 400, 400)).toEqual({ start: false, end: false });
	});
	it('reads a right-to-left row, whose position counts down from 0', () => {
		expect(edgesOf(-300, 1000, 400)).toEqual({ start: true, end: true });
		expect(edgesOf(-600, 1000, 400)).toEqual({ start: true, end: false });
	});
});
