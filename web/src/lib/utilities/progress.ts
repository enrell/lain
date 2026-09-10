import type { CatalogItem, Progress } from '$lib/api/types';

const MIN_RESUME_SEC = 5;
const COMPLETED_RATIO = 0.95;

/** Whether a progress record should appear in Continue Watching. */
export function isInProgress(p: Progress): boolean {
	if (p.item_id === '') return false;
	if (p.completed) return false;
	return p.position_sec >= MIN_RESUME_SEC;
}

/** Continue Watching order: most recently touched first. */
export function sortByRecent<T extends { updated_at: number }>(items: T[]): T[] {
	return [...items].sort((a, b) => b.updated_at - a.updated_at);
}

/**
 * Resume point for a progress record. Returns 0 when there is nothing
 * meaningful to resume (never started, finished, or inside the first
 * seconds / last seconds of the media).
 */
export function resumePosition(p: Progress | null, duration: number): number {
	if (!p || p.item_id === '' || p.completed) return 0;
	const pos = p.position_sec;
	if (!Number.isFinite(pos) || pos < MIN_RESUME_SEC) return 0;
	if (Number.isFinite(duration) && duration > 0) {
		if (pos >= duration * COMPLETED_RATIO) return 0;
		if (duration - pos < 10) return 0;
	} else if (Number.isFinite(p.duration_sec) && p.duration_sec > 0) {
		if (pos >= p.duration_sec * COMPLETED_RATIO) return 0;
	}
	return pos;
}

/** Completed rule used by the player when writing progress. */
export function isCompleted(position: number, duration: number): boolean {
	return Number.isFinite(duration) && duration > 0 && position / duration >= COMPLETED_RATIO;
}

export function progressRatio(p: Progress): number {
	if (!p.duration_sec || p.duration_sec <= 0) return 0;
	return Math.max(0, Math.min(1, p.position_sec / p.duration_sec));
}
