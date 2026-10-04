import type { DownloadJob, DownloadSettings, DownloadState } from '$lib/api';

const GiB = 1024 ** 3;

/** Bytes shown as whole-or-decimal GiB in a number input. */
export function toGiB(bytes: number): string {
	if (!Number.isFinite(bytes) || bytes <= 0) return '0';
	const v = bytes / GiB;
	return Number.isInteger(v) ? String(v) : v.toFixed(2).replace(/0+$/, '').replace(/\.$/, '');
}

/** A GiB input back to bytes; empty, negative or junk is 0 (no limit). */
export function fromGiB(raw: string): number {
	const v = Number.parseFloat(raw);
	if (!Number.isFinite(v) || v <= 0) return 0;
	return Math.round(v * GiB);
}

/** Jobs that still move bytes or wait to. */
export function isActive(state: DownloadState): boolean {
	return state === 'queued' || state === 'running';
}

/** 0..1, 0 while the total is unknown. */
export function progressRatio(job: Pick<DownloadJob, 'bytes' | 'total' | 'state'>): number {
	if (job.state === 'done') return 1;
	if (job.total <= 0) return 0;
	return Math.min(1, Math.max(0, job.bytes / job.total));
}

/** How many staged fields differ from the saved settings. */
export function settingsChanges(saved: DownloadSettings, draft: DownloadSettings): number {
	const keys: (keyof DownloadSettings)[] = ['dir', 'max_bytes', 'min_free_bytes', 'concurrency', 'keep_finished_days'];
	return keys.filter((k) => String(saved[k]) !== String(draft[k])).length;
}
