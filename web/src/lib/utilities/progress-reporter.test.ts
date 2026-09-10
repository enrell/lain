import { describe, expect, it, vi } from 'vitest';
import { ProgressReporter, type ProgressSnapshot } from './progress-reporter';

function snap(position: number, duration = 100): ProgressSnapshot {
	return { position_sec: position, duration_sec: duration, completed: false };
}

describe('ProgressReporter', () => {
	it('does not write before the interval elapses', async () => {
		const sends: ProgressSnapshot[] = [];
		let now = 1_000;
		const reporter = new ProgressReporter(
			async (s) => {
				sends.push(s);
			},
			{ now: () => now, intervalMs: 10_000 }
		);
		await reporter.update(snap(1));
		await reporter.update(snap(4));
		expect(sends).toHaveLength(0);
		now += 10_000;
		await reporter.update(snap(15));
		expect(sends).toHaveLength(1);
		expect(sends[0].position_sec).toBe(15);
	});

	it('forces writes on lifecycle transitions', async () => {
		const sends: ProgressSnapshot[] = [];
		const reporter = new ProgressReporter(
			async (s) => {
				sends.push(s);
			},
			{ now: () => 0 }
		);
		await reporter.update(snap(2), { force: true });
		expect(sends).toHaveLength(1);
	});

	it('coalesces writes while one is in flight', async () => {
		const sends: ProgressSnapshot[] = [];
		let release!: () => void;
		const gate = new Promise<void>((resolve) => (release = resolve));
		const reporter = new ProgressReporter(
			async (s) => {
				sends.push(s);
				if (s.position_sec === 1) await gate;
			},
			{ now: () => 0 }
		);
		void reporter.update(snap(1), { force: true });
		void reporter.update(snap(2), { force: true });
		void reporter.update(snap(3), { force: true });
		release();
		await vi.waitFor(() => expect(sends.length).toBe(2));
		expect(sends[0].position_sec).toBe(1);
		expect(sends[1].position_sec).toBe(3);
	});

	it('keeps a failed write pending and retries on the next flush', async () => {
		const sends: ProgressSnapshot[] = [];
		let fail = true;
		const errors: unknown[] = [];
		const reporter = new ProgressReporter(
			async (s) => {
				sends.push(s);
				if (fail) throw new Error('offline');
			},
			{ now: () => 0, onError: (err) => errors.push(err) }
		);
		await reporter.update(snap(7), { force: true });
		expect(sends).toHaveLength(1);
		expect(errors).toHaveLength(1);
		expect(reporter.pending).toBe(true);
		fail = false;
		await reporter.flush();
		expect(sends).toHaveLength(2);
		expect(sends[1].position_sec).toBe(7);
		expect(reporter.pending).toBe(false);
	});
});
