export interface ProgressSnapshot {
	position_sec: number;
	duration_sec: number;
	completed: boolean;
}

export interface ProgressReporterOptions {
	/** Minimum time between automatic writes while playing. */
	intervalMs?: number;
	/** Minimum position change worth writing. */
	minDeltaSec?: number;
	now?: () => number;
	onError?: (err: unknown) => void;
}

/**
 * Bounded progress persistence: writes at most once per interval while
 * playing, plus forced writes on lifecycle transitions (pause, seek,
 * ended, page hide). Concurrent writes are coalesced — only the newest
 * snapshot survives, so a slow server can never build a request queue.
 * A failed write keeps the snapshot pending for the next flush instead
 * of breaking playback.
 */
export class ProgressReporter {
	#send: (snapshot: ProgressSnapshot) => Promise<unknown>;
	#intervalMs: number;
	#minDeltaSec: number;
	#now: () => number;
	#onError?: (err: unknown) => void;

	#latest: ProgressSnapshot | null = null;
	#sending = false;
	#last = { pos: 0, at: 0 };

	constructor(
		send: (snapshot: ProgressSnapshot) => Promise<unknown>,
		opts: ProgressReporterOptions = {}
	) {
		this.#send = send;
		this.#intervalMs = opts.intervalMs ?? 10_000;
		this.#minDeltaSec = opts.minDeltaSec ?? 5;
		this.#now = opts.now ?? (() => Date.now());
		this.#onError = opts.onError;
	}

	/** Called from timeupdate (not forced) and lifecycle events (forced). */
	update(snapshot: ProgressSnapshot, opts: { force?: boolean } = {}): Promise<void> {
		this.#latest = snapshot;
		if (opts.force) return this.flush();
		const elapsed = this.#now() - this.#last.at;
		const moved = Math.abs(snapshot.position_sec - this.#last.pos);
		if (elapsed >= this.#intervalMs && moved >= this.#minDeltaSec) {
			return this.flush();
		}
		return Promise.resolve();
	}

	/** Sends the pending snapshot now (coalesced). */
	async flush(): Promise<void> {
		if (this.#sending || this.#latest === null) return;
		const snapshot = this.#latest;
		this.#latest = null;
		this.#sending = true;
		let failed = false;
		try {
			await this.#send(snapshot);
			this.#last = { pos: snapshot.position_sec, at: this.#now() };
		} catch (err) {
			// Keep the snapshot pending for the next flush; never retry in
			// a loop, or a down server turns into a request storm.
			this.#latest ??= snapshot;
			failed = true;
			this.#onError?.(err);
		} finally {
			this.#sending = false;
			// Coalesced writes made while this one was in flight still go
			// out; a failure waits for the caller's next update/flush.
			if (!failed && this.#latest !== null) void this.flush();
		}
	}

	get pending(): boolean {
		return this.#latest !== null || this.#sending;
	}
}
