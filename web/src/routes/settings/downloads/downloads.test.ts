import { describe, expect, it } from 'vitest';
import { fromGiB, isActive, progressRatio, settingsChanges, toGiB } from './downloads';

const base = { dir: '/srv/dl', max_bytes: 20 * 1024 ** 3, min_free_bytes: 5 * 1024 ** 3, concurrency: 2, keep_finished_days: 30 };

describe('download settings helpers', () => {
	it('round-trips GiB inputs and treats junk as no limit', () => {
		expect(toGiB(20 * 1024 ** 3)).toBe('20');
		expect(toGiB(1.5 * 1024 ** 3)).toBe('1.5');
		expect(toGiB(0)).toBe('0');
		expect(fromGiB('20')).toBe(20 * 1024 ** 3);
		expect(fromGiB('0.5')).toBe(512 * 1024 ** 2);
		for (const junk of ['', '-3', 'lots', 'NaN']) expect(fromGiB(junk)).toBe(0);
	});
	it('counts staged changes field by field', () => {
		expect(settingsChanges(base, { ...base })).toBe(0);
		expect(settingsChanges(base, { ...base, dir: '/x', concurrency: 3 })).toBe(2);
		// A number input may hand back a string; equal values are not changes.
		expect(settingsChanges(base, { ...base, concurrency: '2' as unknown as number })).toBe(0);
	});
	it('reports progress and activity', () => {
		expect(progressRatio({ bytes: 50, total: 200, state: 'running' })).toBe(0.25);
		expect(progressRatio({ bytes: 50, total: -1, state: 'running' })).toBe(0);
		expect(progressRatio({ bytes: 0, total: -1, state: 'done' })).toBe(1);
		expect(progressRatio({ bytes: 300, total: 200, state: 'running' })).toBe(1);
		expect(isActive('running') && isActive('queued')).toBe(true);
		expect(isActive('paused') || isActive('done') || isActive('failed')).toBe(false);
	});
});
