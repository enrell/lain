/*
 * Where a sideways-scrolling row hides more: before its start once it has
 * moved, after its end until that is reached. A rail shows its previous
 * and next buttons by it (Rail), and a row of names fades at those edges
 * only (`edge-fade` in app.css): a fade at an edge with nothing past it
 * read as a cut name.
 */
import type { Attachment } from 'svelte/attachments';

export interface Edges {
	/** Something is hidden before the row's start (left, or right in a right-to-left page). */
	start: boolean;
	/** Something is hidden after its end. */
	end: boolean;
}

/** Edges from the scroll position; `scrollLeft` counts down from 0 in a right-to-left row. */
export function edgesOf(scrollLeft: number, scrollWidth: number, clientWidth: number): Edges {
	const moved = Math.abs(scrollLeft);
	const max = scrollWidth - clientWidth;
	// A pixel of slack: zoomed pages report fractional positions.
	return { start: moved > 1, end: max - moved > 1 };
}

/**
 * Follows a row's edges as it scrolls, resizes or changes its children.
 * With `fade`, sets `--fade-left` and `--fade-right` for `edge-fade` (on
 * the physical sides, flipped in a right-to-left row).
 */
export function scrollEdges(onChange?: (edges: Edges) => void, fade = ''): Attachment<HTMLElement> {
	return (el) => {
		let last = '';
		const update = (): void => {
			const edges = edgesOf(el.scrollLeft, el.scrollWidth, el.clientWidth);
			const key = `${edges.start}:${edges.end}`;
			if (key === last) return;
			last = key;
			if (fade) {
				const rtl = getComputedStyle(el).direction === 'rtl';
				el.style.setProperty('--fade-left', (rtl ? edges.end : edges.start) ? fade : '0px');
				el.style.setProperty('--fade-right', (rtl ? edges.start : edges.end) ? fade : '0px');
			}
			onChange?.(edges);
		};
		update();
		el.addEventListener('scroll', update, { passive: true });
		const resized = new ResizeObserver(update);
		resized.observe(el);
		// Cards drawn later (a rail's pictures, a shelf filling in) change the width.
		const changed = new MutationObserver(update);
		changed.observe(el, { childList: true, subtree: true });
		return () => {
			el.removeEventListener('scroll', update);
			resized.disconnect();
			changed.disconnect();
		};
	};
}
