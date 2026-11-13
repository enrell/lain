import { t } from '$lib/i18n';

/** Catalog kinds that open in the reader instead of the video player (D-085). */
const READABLE = new Set(['comic', 'manga']);

export function isReadable(kind: string | undefined | null): boolean {
	return !!kind && READABLE.has(kind);
}

/**
 * How a readable file is named in the UI. The catalog stores a volume as
 * the season and a chapter or issue as the episode, each 0 when the file
 * does not carry it, so a volume file, a chapter file and "Vol 2 Ch 15"
 * stay distinguishable.
 */
export function readingLabel(item: {
	kind: string;
	season: number;
	episode: number;
	title: string;
}): string {
	const manga = item.kind === 'manga';
	const volume = item.season;
	const number = item.episode;
	if (volume > 0 && number > 0)
		return manga ? t('reader.label.volumeChapter', { volume, number }) : t('reader.label.volumeIssue', { volume, number });
	if (volume > 0) return t('reader.label.volume', { volume });
	if (number > 0) return manga ? t('reader.label.chapter', { number }) : t('reader.label.issue', { number });
	return item.title;
}
