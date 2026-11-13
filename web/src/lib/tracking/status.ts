import { t } from '$lib/i18n';
import type { ListEntry } from '$lib/api/types';

/*
 * The list vocabulary the server normalizes every platform onto (D-078),
 * in AniList's order, and how a list's words read on screen: an entry
 * that is read says Reading and Rereading where a watched one says
 * Watching and Rewatching.
 */
export const STATUSES = ['current', 'planning', 'completed', 'paused', 'dropped', 'repeating'] as const;
export type ListStatus = (typeof STATUSES)[number];

/** Media kinds that are read, not watched. */
export function isReadingMedia(mediaType: string | undefined): boolean {
	return mediaType === 'manga' || mediaType === 'comic';
}

export function statusLabel(status: string, reading: boolean): string {
	switch (status) {
		case 'current':
			return t(reading ? 'tracking.status.currentReading' : 'tracking.status.current');
		case 'repeating':
			return t(reading ? 'tracking.status.repeatingReading' : 'tracking.status.repeating');
		case 'planning':
			return t('tracking.status.planning');
		case 'completed':
			return t('tracking.status.completed');
		case 'paused':
			return t('tracking.status.paused');
		case 'dropped':
			return t('tracking.status.dropped');
		default:
			return status;
	}
}

/** A platform's own spelling of its name. */
export function platformName(id: string): string {
	return id === 'anilist' ? 'AniList' : id;
}

/** "12/28 ep", "40 ch · vol 3": how far the entry is, in its own units. */
export function progressText(e: Pick<ListEntry, 'media_type' | 'progress' | 'progress_total' | 'progress_volumes'>): string {
	const reading = isReadingMedia(e.media_type);
	const params = { progress: e.progress, total: e.progress_total ?? 0 };
	const main = e.progress_total
		? t(reading ? 'tracking.progressChapters' : 'tracking.progressEpisodes', params)
		: t(reading ? 'tracking.progressChaptersOpen' : 'tracking.progressEpisodesOpen', params);
	return e.progress_volumes ? `${main} · ${t('tracking.progressVolumes', { volumes: e.progress_volumes })}` : main;
}

/** Where Acquire's search for an entry starts (admin): its title, in the matching library kind. */
export function acquireHref(e: Pick<ListEntry, 'title' | 'media_type'>): string {
	const q = new URLSearchParams({ q: e.title, kind: isReadingMedia(e.media_type) ? 'manga' : 'anime' });
	return `/acquire?${q.toString()}`;
}
