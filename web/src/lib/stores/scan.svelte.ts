import { api } from '$lib/api';
import type { ScanStatus } from '$lib/api/types';
import { i18n, t } from '$lib/i18n';
import { errorMessage } from '$lib/utilities/errors';
import { elapsedSince, formatTime } from '$lib/utilities/format';

const POLL_MS = 1200;

/**
 * Bounded scan polling: one timer while a scan is running, cancelled on
 * terminal state (done/error/idle) and on stop() when the shell
 * unmounts. The shell owns the timer, so the header and every page
 * read the same live status; a page leaving never freezes it.
 * No WebSockets — the backend contract is a status endpoint.
 */
class ScanStore {
	status = $state<ScanStatus | null>(null);
	loading = $state(false);
	error = $state<string | null>(null);
	/** Wall clock of the last poll, for the elapsed time. */
	now = $state(Date.now());

	#timer: ReturnType<typeof setInterval> | null = null;

	async refresh(): Promise<void> {
		this.loading = true;
		try {
			this.status = await api.libraries.scanStatus();
			this.now = Date.now();
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

	/** Read the status once and keep polling if a scan is running. */
	async watch(): Promise<void> {
		await this.refresh();
		if (this.running) this.follow();
	}

	/** Begin polling while a scan runs; safe to call repeatedly. */
	follow(): void {
		if (this.#timer !== null) return;
		this.#timer = setInterval(() => void this.refresh(), POLL_MS);
	}

	get running(): boolean {
		return this.status?.state === 'running';
	}

	/** "m:ss" since the running scan started; '' when unknown. */
	get elapsed(): string {
		const started = this.running ? this.status?.started_at : undefined;
		return started ? formatTime(elapsedSince(started, this.now)) : '';
	}

	/** What the running scan is doing, in one line; '' when none runs. */
	get line(): string {
		if (!this.running) return '';
		const p = this.status?.progress;
		if (!p) return t('scan.running');
		const n = (v: number | undefined) => i18n.formatNumber(v ?? 0);
		const libs = { walked: n(p.libraries_walked), libraries: n(p.libraries) };
		switch (p.phase) {
			case 'walk':
				return t('scan.walk', libs);
			case 'identify': {
				const files = { done: n(p.files_done), total: n(p.files) };
				return p.libraries_walked < p.libraries
					? t('scan.identifyWalking', { ...files, ...libs })
					: t('scan.identify', files);
			}
			case 'commit':
				return t('scan.commit', { done: n(p.libraries_done), libraries: n(p.libraries) });
			case 'enrich':
				return p.enrich_total
					? t('scan.enrich', { done: n(p.enrich_done), total: n(p.enrich_total) })
					: t('scan.enrichStarting');
			default:
				return t('scan.running');
		}
	}

	/**
	 * How far the current phase is, 0..1, or null when it cannot be
	 * known yet (folders still being listed, a single commit).
	 */
	get fraction(): number | null {
		const p = this.running ? this.status?.progress : undefined;
		if (!p) return null;
		const of = (done: number | undefined, total: number | undefined) =>
			total ? Math.min(1, (done ?? 0) / total) : null;
		switch (p.phase) {
			case 'identify':
				// The total still grows while other roots are being listed.
				return p.libraries_walked < p.libraries ? null : of(p.files_done, p.files);
			case 'commit':
				return p.libraries > 1 ? of(p.libraries_done, p.libraries) : null;
			case 'enrich':
				return of(p.enrich_done, p.enrich_total);
			default:
				return null;
		}
	}

	stop(): void {
		if (this.#timer !== null) {
			clearInterval(this.#timer);
			this.#timer = null;
		}
	}
}

export const scan = new ScanStore();
