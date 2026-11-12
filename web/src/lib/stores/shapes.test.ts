import { describe, expect, it } from 'vitest';
import { recordShape, shapeOf } from './shapes.svelte';

describe('shapes', () => {
	it('uses the guess until the picture is measured', () => {
		expect(shapeOf('a', 'landscape')).toBe('landscape');
		recordShape('a', 400, 600);
		expect(shapeOf('a', 'landscape')).toBe('portrait');
		recordShape('b', 1280, 720);
		expect(shapeOf('b', 'portrait')).toBe('landscape');
	});
	it('treats a near-square picture as portrait and ignores a broken one', () => {
		recordShape('c', 500, 480);
		expect(shapeOf('c', 'landscape')).toBe('portrait');
		recordShape('d', 0, 0);
		expect(shapeOf('d', 'landscape')).toBe('landscape');
	});
});
