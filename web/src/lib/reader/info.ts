import type { ComicInfo } from '$lib/api/types';

/** Which ComicInfo fields the title page lists, in order. */
export type InfoField = 'series' | 'number' | 'volume' | 'writer' | 'artist' | 'publisher' | 'year' | 'language';

export interface InfoRow {
	field: InfoField;
	value: string;
}

/**
 * The facts worth a row, skipping what the archive did not say. Summary,
 * genres and direction render separately (prose, badges, a toggle hint).
 */
export function infoRows(info: ComicInfo | undefined | null): InfoRow[] {
	if (!info) return [];
	const rows: InfoRow[] = [];
	const push = (field: InfoField, v: string | number | undefined) => {
		if (v !== undefined && v !== '' && v !== 0) rows.push({ field, value: String(v) });
	};
	push('series', info.series);
	push('number', info.number && info.count ? `${info.number} / ${info.count}` : info.number);
	push('volume', info.volume);
	push('writer', info.writer);
	push('artist', info.artist);
	push('publisher', info.publisher);
	push('year', info.year);
	push('language', info.language);
	return rows;
}

/** Where the reading direction comes from: the archive or the library type. */
export function directionSource(info: ComicInfo | undefined | null): 'archive' | 'library' {
	return info?.direction ? 'archive' : 'library';
}
