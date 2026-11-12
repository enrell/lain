/*
 * The player's personal choices kept in this browser: how far one skip
 * goes and how subtitles look. The player's own menu and Settings >
 * Playback read and write the same keys, so a choice made in one is the
 * choice in force in the other (the app keeps the same three in its
 * Settings).
 */
import { prefs } from '$lib/auth/storage';

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
