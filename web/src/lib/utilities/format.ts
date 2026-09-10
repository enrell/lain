import type { CatalogItem } from '$lib/api/types';

/** h:mm:ss for long media, m:ss below an hour. */
export function formatTime(seconds: number | null | undefined): string {
	if (seconds === null || seconds === undefined || !Number.isFinite(seconds) || seconds < 0) {
		return '--:--';
	}
	const total = Math.floor(seconds);
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = total % 60;
	const mm = h > 0 ? String(m).padStart(2, '0') : String(m);
	const ss = String(s).padStart(2, '0');
	return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

/** "1 h 42 min" for metadata lines (never a clock). */
export function formatRuntime(seconds: number | null | undefined): string {
	if (!seconds || !Number.isFinite(seconds) || seconds <= 0) return '';
	const total = Math.round(seconds / 60);
	const h = Math.floor(total / 60);
	const m = total % 60;
	if (h === 0) return `${m} min`;
	return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

export function formatBytes(bytes: number | null | undefined): string {
	if (!bytes || !Number.isFinite(bytes) || bytes <= 0) return '';
	const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
	let value = bytes;
	let unit = 0;
	while (value >= 1024 && unit < units.length - 1) {
		value /= 1024;
		unit++;
	}
	const digits = value >= 100 || unit === 0 ? 0 : 1;
	return `${value.toFixed(digits)} ${units[unit]}`;
}

export function formatDate(unixSeconds: number | null | undefined): string {
	if (!unixSeconds) return '';
	return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
		year: 'numeric',
		month: 'short',
		day: 'numeric'
	});
}

export function formatRelative(unixSeconds: number | null | undefined): string {
	if (!unixSeconds) return '';
	const diff = Date.now() / 1000 - unixSeconds;
	if (diff < 60) return 'just now';
	if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
	if (diff < 86400) return `${Math.floor(diff / 3600)} h ago`;
	if (diff < 86400 * 30) return `${Math.floor(diff / 86400)} d ago`;
	return formatDate(unixSeconds);
}

/** "S02E05", "2023", or the library kind — whatever actually exists. */
export function mediaSubtitle(item: CatalogItem): string {
	const parts: string[] = [];
	if (item.season > 0 && item.episode > 0) {
		parts.push(`S${String(item.season).padStart(2, '0')}E${String(item.episode).padStart(2, '0')}`);
	} else if (item.episode > 0) {
		parts.push(`EP ${item.episode}`);
	}
	if (item.year > 0) parts.push(String(item.year));
	if (parts.length === 0 && item.kind) parts.push(item.kind);
	return parts.join(' · ');
}

export function plural(n: number, one: string, many: string): string {
	return n === 1 ? one : many;
}

/** Bytes per second for playback debug labels; not shown by default. */
export function formatRate(bytesPerSecond: number | null | undefined): string {
	if (!bytesPerSecond || !Number.isFinite(bytesPerSecond)) return '';
	return `${formatBytes(bytesPerSecond)}/s`;
}
