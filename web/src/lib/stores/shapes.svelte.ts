import { SvelteMap } from 'svelte/reactivity';

/**
 * The shape of each title's picture, learned when it loads. A card takes
 * the shape of what it shows (a portrait cover or a landscape still), so a
 * library that holds both never forces one into the other's frame.
 */
export type Shape = 'portrait' | 'landscape';

const measured = new SvelteMap<string, Shape>();

export function recordShape(id: string, width: number, height: number): void {
	if (width <= 0 || height <= 0) return;
	const shape: Shape = width > height * 1.15 ? 'landscape' : 'portrait';
	if (measured.get(id) !== shape) measured.set(id, shape);
}

/** The measured shape, or `guess` until the picture has loaded. */
export function shapeOf(id: string, guess: Shape): Shape {
	return measured.get(id) ?? guess;
}

/** Width over height of the frame each shape gets. */
export const RATIO: Record<Shape, number> = { portrait: 2 / 3, landscape: 16 / 9 };
