/*
 * The player's personal choices kept in this browser: how far one skip
 * goes and how subtitles look. The player's own menu and Settings >
 * Playback read and write the same keys, so a choice made in one is the
 * choice in force in the other (the app keeps the same three in its
 * Settings). Also the audio and subtitle language last picked for a
 * title, which win over the account's preferences when its next part
 * opens (D-280; the app keeps the same per title).
 */
import { prefs } from '$lib/auth/storage';
import type { RememberedTracks } from './track-language';

/** Seconds one skip goes; 0 lets the player pick by the title's length. */
export const SKIP_CHOICES = [0, 5, 10, 30, 60] as const;
export type SubtitleSize = 'small' | 'medium' | 'large';
export type SubtitleBackground = 'none' | 'box';

const SKIP = 'player.skip';
const SUBTITLE_SIZE = 'player.subtitle_size';
const SUBTITLE_BG = 'player.subtitle_bg';

export function loadSkip(): number {
	const n = Number(prefs.get(SKIP));
	return (SKIP_CHOICES as readonly number[]).includes(n) ? n : 0;
}

/** Writes are best effort: without storage the choice lasts for this visit. */
export function saveSkip(seconds: number): void {
	try {
		prefs.set(SKIP, String(seconds));
	} catch {
		/* storage full or blocked */
	}
}

export function loadSubtitleSize(): SubtitleSize {
	const v = prefs.get(SUBTITLE_SIZE);
	return v === 'small' || v === 'large' ? v : 'medium';
}

export function saveSubtitleSize(size: SubtitleSize): void {
	try {
		prefs.set(SUBTITLE_SIZE, size);
	} catch {
		/* storage full or blocked */
	}
}

export function loadSubtitleBackground(): SubtitleBackground {
	return prefs.get(SUBTITLE_BG) === 'box' ? 'box' : 'none';
}

export function saveSubtitleBackground(bg: SubtitleBackground): void {
	try {
		prefs.set(SUBTITLE_BG, bg);
	} catch {
		/* storage full or blocked */
	}
}

/** One key per title: its name as the catalog writes it, case aside, so every episode shares it. */
const titleKey = (kind: 'audio' | 'subtitle', title: string) => `player.tracks.${kind}.${title.trim().toLowerCase()}`;

export function loadTitleTracks(title: string): RememberedTracks {
	if (!title.trim()) return {};
	const audio = prefs.get(titleKey('audio', title));
	const subtitle = prefs.get(titleKey('subtitle', title));
	return { ...(audio ? { audio } : {}), ...(subtitle !== null ? { subtitle } : {}) };
}

/**
 * Remembers a pick for this title: a language, or `null` for subtitles
 * off. A track with no language tag says nothing to keep.
 */
export function saveTitleTrack(title: string, kind: 'audio' | 'subtitle', language: string | null | undefined): void {
	const lang = language === null ? '' : (language ?? '').toLowerCase();
	if (!title.trim() || (language !== null && (!lang || lang === 'und'))) return;
	if (kind === 'audio' && !lang) return;
	try {
		prefs.set(titleKey(kind, title), lang);
	} catch {
		/* storage full or blocked */
	}
}
