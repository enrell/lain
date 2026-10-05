import { languageMatches } from './track-language';

/*
 * Sidecar subtitles (docs/slices/acquisition.md, A-29): files next to the
 * media, listed by GET /api/items/{id}/sidecars and served as WebVTT.
 * They share the player's subtitle menu with embedded tracks; their
 * option values carry a prefix so an embedded stream index never reaches
 * a sidecar URL and a sidecar never reaches a transcode request.
 */

export interface SidecarTrack {
	index: number;
	name: string;
	language: string; // ISO 639-2/T or 'und'
	tag?: string;
	forced?: boolean;
	hi?: boolean;
	format: string;
}

const PREFIX = 'sc:';

export function sidecarValue(index: number): string {
	return PREFIX + index;
}

/** The sidecar index of a menu choice, or null for embedded/off. */
export function sidecarIndex(choice: string): number | null {
	if (!choice.startsWith(PREFIX)) return null;
	const n = Number(choice.slice(PREFIX.length));
	return Number.isInteger(n) && n >= 0 ? n : null;
}

/** The embedded stream part of a choice ('' when a sidecar or off). */
export function embeddedChoice(choice: string): string {
	return choice.startsWith(PREFIX) ? '' : choice;
}

/** Menu label: the language tag as written, marked as a file. */
export function sidecarLabel(s: SidecarTrack): string {
	const parts = [s.tag || (s.language !== 'und' ? s.language : s.name), 'file'];
	if (s.forced) parts.push('forced');
	if (s.hi) parts.push('SDH');
	return parts.join(' · ');
}

/**
 * D-071 extended to sidecars: when audio is not in the account language
 * and no embedded subtitle matches, show a matching full (not forced)
 * sidecar.
 */
export function pickSidecar(
	sidecars: SidecarTrack[],
	language: string,
	state: { audioMatches: boolean; embeddedFound: boolean }
): number | undefined {
	if (!language || state.audioMatches || state.embeddedFound) return undefined;
	return sidecars.find((s) => !s.forced && languageMatches(s.language, language))?.index;
}
