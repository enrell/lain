import { api } from '$lib/api';
import type { ScanStatus } from '$lib/api/types';
import { errorMessage } from '$lib/utilities/errors';

const POLL_MS = 1200;

/**
 * Bounded scan polling: one timer while a scan is running, cancelled on
 * terminal state (done/error/idle) and on stop() when the page unmounts.
 * No WebSockets — the backend contract is a status endpoint.
 */
class ScanStore {
	status = $state<ScanStatus | null>(null);
	loading = $state(false);
	error = $state<string | null>(null);

	#timer: ReturnType<typeof setInterval> | null = null;

	async refresh(): Promise<void> {
		this.loading = true;
		try {
			this.status = await api.libraries.scanStatus();
			this.error = null;
			if (this.status.state !== 'running') this.stop();
		} catch (err) {
			this.error = errorMessage(err);
			this.stop();
		} finally {
			this.loading = false;
		}
	}

	async start(): Promise<void> {
		this.error = null;
		await api.libraries.scanStart();
		await this.refresh();
		this.follow();
	}

	/** Begin polling while a scan runs; safe to call repeatedly. */
	follow(): void {
		if (this.#timer !== null) return;
		this.#timer = setInterval(() => void this.refresh(), POLL_MS);
	}

	get running(): boolean {
		return this.status?.state === 'running';
	}

	stop(): void {
		if (this.#timer !== null) {
			clearInterval(this.#timer);
			this.#timer = null;
		}
	}
}

export const scan = new ScanStore();
