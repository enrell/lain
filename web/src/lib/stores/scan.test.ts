import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ScanStatus } from '$lib/api/types';

const calls = vi.hoisted(() => ({ starts: 0, status: { state: 'running' } as ScanStatus, conflict: false }));

vi.mock('$lib/api', async () => {
	const { ApiError } = await import('$lib/api/client');
	return {
		ApiError,
		api: {
			libraries: {
				scanStart: async () => {
					calls.starts += 1;
					if (calls.conflict) throw new ApiError('conflict', 'scan already running', 409);
					calls.status = { state: 'running' } as ScanStatus;
					return { state: 'running' };
				},
				scanStatus: async () => calls.status
			}
		}
	};
});

const { scan } = await import('./scan.svelte');

beforeEach(() => {
	vi.useFakeTimers();
	calls.starts = 0;
	calls.status = { state: 'running' } as ScanStatus;
	calls.conflict = false;
	scan.queued = false;
	scan.status = null;
});

afterEach(() => {
	scan.stop();
	vi.useRealTimers();
});

describe('scan store', () => {
	it('starts a scan when none runs', async () => {
		expect(await scan.start()).toBe('started');
		expect(calls.starts).toBe(1);
		expect(scan.queued).toBe(false);
	});

	it('queues a scan asked for while one runs and starts it when that one ends', async () => {
		calls.conflict = true;
		expect(await scan.start()).toBe('queued');
		expect(scan.queued).toBe(true);
		expect(calls.starts).toBe(1);

		// The running scan ends; the next poll starts the queued one.
		calls.conflict = false;
		calls.status = { state: 'done' } as ScanStatus;
		await vi.advanceTimersByTimeAsync(1300);
		expect(calls.starts).toBe(2);
		expect(scan.queued).toBe(false);
		expect(scan.running).toBe(true);
	});

	it('does not queue when the running scan ended before the status was read', async () => {
		calls.conflict = true;
		calls.status = { state: 'done' } as ScanStatus;
		const how = scan.start();
		// The refresh after the 409 sees no scan running and starts one at once.
		calls.conflict = false;
		expect(await how).toBe('started');
		await vi.advanceTimersByTimeAsync(0);
		expect(calls.starts).toBe(2);
		expect(scan.queued).toBe(false);
	});
});
