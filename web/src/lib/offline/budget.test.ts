import { describe, expect, it } from 'vitest';
import { DEFAULT_CAP, effectiveBudget, fits, loadCap, saveCap } from './budget';

const GiB = 1024 ** 3;

describe('offline budget', () => {
	it('takes the smaller of the cap and the browser headroom', () => {
		expect(effectiveBudget(GiB, {}, 0)).toBe(GiB);
		expect(effectiveBudget(GiB, { quota: 10 * GiB, usage: 0 }, 0)).toBe(GiB);
		// 1 GiB quota: 80% share, minus 300 MiB that is not ours.
		expect(effectiveBudget(5 * GiB, { quota: GiB, usage: 400 * 1024 ** 2 }, 100 * 1024 ** 2)).toBe(0.8 * GiB - 300 * 1024 ** 2);
		expect(effectiveBudget(0, { quota: 2 * GiB, usage: 0 }, 0)).toBe(1.6 * GiB);
		expect(effectiveBudget(0, {}, 0)).toBe(Number.POSITIVE_INFINITY);
		expect(effectiveBudget(GiB, { quota: 100, usage: 1000 }, 0)).toBe(0);
	});
	it('checks a save against what is already kept', () => {
		expect(fits(10, 90, 100)).toBe(true);
		expect(fits(11, 90, 100)).toBe(false);
	});
	it('persists the cap and falls back on junk', () => {
		const m = new Map<string, string>();
		const kv = { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
		expect(loadCap(kv)).toBe(DEFAULT_CAP);
		saveCap(2 * GiB, kv);
		expect(loadCap(kv)).toBe(2 * GiB);
		saveCap(0, kv);
		expect(loadCap(kv)).toBe(0);
		kv.setItem('lain.offline.capBytes', 'lots');
		expect(loadCap(kv)).toBe(DEFAULT_CAP);
		expect(loadCap(null)).toBe(DEFAULT_CAP);
	});
});
