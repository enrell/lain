import { describe, expect, it } from 'vitest';
import type { Progress } from '$lib/api/types';
import { isCompleted, isInProgress, progressRatio, resumePosition, sortByRecent } from './progress';

function progress(partial: Partial<Progress>): Progress {
	return {
		item_id: 'i1',
		user_id: 'u1',
		position_sec: 0,
		duration_sec: 0,
		completed: false,
		updated_at: 0,
		...partial
	};
}

describe('isInProgress', () => {
	it('keeps unfinished items past the minimum resume point', () => {
		expect(isInProgress(progress({ position_sec: 30 }))).toBe(true);
		expect(isInProgress(progress({ position_sec: 2 }))).toBe(false);
		expect(isInProgress(progress({ position_sec: 30, completed: true }))).toBe(false);
		expect(isInProgress(progress({ item_id: '' }))).toBe(false);
	});
});

describe('resumePosition', () => {
	it('returns 0 without a record', () => {
		expect(resumePosition(null, 600)).toBe(0);
	});
	it('returns 0 for tiny positions and completed records', () => {
		expect(resumePosition(progress({ position_sec: 3 }), 600)).toBe(0);
		expect(resumePosition(progress({ position_sec: 300, completed: true }), 600)).toBe(0);
	});
	it('resumes mid-media', () => {
		expect(resumePosition(progress({ position_sec: 300 }), 600)).toBe(300);
	});
	it('does not resume within the final seconds', () => {
		expect(resumePosition(progress({ position_sec: 595 }), 600)).toBe(0);
	});
	it('uses recorded duration when the real duration is unknown', () => {
		expect(resumePosition(progress({ position_sec: 300, duration_sec: 600 }), 0)).toBe(300);
		expect(resumePosition(progress({ position_sec: 590, duration_sec: 600 }), 0)).toBe(0);
	});
});

describe('isCompleted', () => {
	it('treats >= 95% as completed', () => {
		expect(isCompleted(570, 600)).toBe(true);
		expect(isCompleted(569, 600)).toBe(false);
		expect(isCompleted(10, 0)).toBe(false);
	});
});

describe('progressRatio', () => {
	it('clamps to [0,1]', () => {
		expect(progressRatio(progress({ position_sec: 30, duration_sec: 60 }))).toBeCloseTo(0.5);
		expect(progressRatio(progress({ position_sec: 90, duration_sec: 60 }))).toBe(1);
		expect(progressRatio(progress({ position_sec: 30, duration_sec: 0 }))).toBe(0);
	});
});

describe('sortByRecent', () => {
	it('orders most recently updated first without mutating input', () => {
		const a = progress({ item_id: 'a', updated_at: 1 });
		const b = progress({ item_id: 'b', updated_at: 3 });
		const c = progress({ item_id: 'c', updated_at: 2 });
		const out = sortByRecent([a, b, c]);
		expect(out.map((p) => p.item_id)).toEqual(['b', 'c', 'a']);
		expect(a.updated_at).toBe(1);
	});
});
