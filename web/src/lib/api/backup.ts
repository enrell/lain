import { fetchRaw } from './client';

function filenameFrom(header: string | null): string | null {
	if (!header) return null;
	const match = /filename="?([^";]+)"?/.exec(header);
	return match?.[1] ?? null;
}

function stamp(): string {
	const d = new Date();
	const pad = (n: number) => String(n).padStart(2, '0');
	return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(
		d.getMinutes()
	)}`;
}

/**
 * GET /api/admin/backup streams a consistent bbolt snapshot. The
 * browser cannot download an authenticated URL by navigation (no
 * Authorization header), so fetch the bytes and hand them to a
 * temporary object URL. Filename comes from Content-Disposition.
 */
export const backup = {
	async download(): Promise<string> {
		const res = await fetchRaw('/api/admin/backup');
		const blob = await res.blob();
		const name = filenameFrom(res.headers.get('Content-Disposition')) ?? `lain-backup-${stamp()}.db`;
		const url = URL.createObjectURL(blob);
		const a = document.createElement('a');
		a.href = url;
		a.download = name;
		document.body.appendChild(a);
		a.click();
		a.remove();
		setTimeout(() => URL.revokeObjectURL(url), 10_000);
		return name;
	}
};
