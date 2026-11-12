import type { MediaStream, User } from '$lib/api/types';

/*
 * Which audio and subtitle tracks the account's preferences pick (D-280).
 * The same rule as the server's lain-watch tracks.rs, the Android app and
 * the mpv script, with the same test cases:
 *
 * - audio: a track in the audio language, else the file's default;
 * - subtitles by mode: `auto` shows the subtitle language when the audio
 *   that plays is not in it; `always` shows it over any audio; `forced`
 *   takes only a forced track (subtitle language, else the audio's, else
 *   untagged); `off` takes none. With no subtitle language, or no language
 *   tag anywhere, `auto` leaves the player's default (none, here).
 *
 * A choice made in the player for this title (`remembered`) wins.
 */

export type SubtitleMode = 'auto' | 'always' | 'forced' | 'off';
export const SUBTITLE_MODES: readonly SubtitleMode[] = ['auto', 'always', 'forced', 'off'];

export interface TrackPreferences {
	audio: string;
	subtitle: string;
	mode: SubtitleMode;
}

/** What the player chose last time for this title; `subtitle: ''` is off. */
export interface RememberedTracks {
	audio?: string;
	subtitle?: string;
}

export interface TrackChoice {
	audioIndex?: number;
	/** An embedded subtitle stream. */
	subtitleIndex?: number;
	/** A sidecar file (its index in the sidecar list). */
	sidecarIndex?: number;
	/** The audio plays the language asked for (not the default standing in). */
	audioMatches: boolean;
	metadataKnown: boolean;
}

/** A sidecar as the rule needs it (see sidecars.ts). */
export interface SidecarCandidate {
	index: number;
	language: string;
	forced?: boolean;
}

/** A subtitle as the rule sees it: an embedded track or a sidecar file. */
export interface SubtitleCandidate {
	language?: string;
	forced?: boolean;
	default?: boolean;
}

const equivalent: Record<string, string> = {
	fre: 'fra', ger: 'deu', chi: 'zho', dut: 'nld',
	gre: 'ell', rum: 'ron', cze: 'ces', slo: 'slk',
	per: 'fas', may: 'msa', alb: 'sqi', arm: 'hye',
	baq: 'eus', bur: 'mya', ice: 'isl', mac: 'mkd'
};

export function languageMatches(actual: string | undefined, preferred: string): boolean {
	const code = (actual ?? '').toLowerCase().split('-')[0];
	const want = preferred.toLowerCase();
	return !!want && !!code && (equivalent[code] ?? code) === (equivalent[want] ?? want);
}

function isMode(v: string | undefined): v is SubtitleMode {
	return !!v && (SUBTITLE_MODES as readonly string[]).includes(v);
}

/**
 * The account's preferences. A server before D-280 sends only
 * `preferred_language`, which is both languages with `auto` (D-071).
 */
export function trackPreferences(user: User | null | undefined): TrackPreferences {
	const audio = user?.preferred_audio_language ?? '';
	const subtitle = user?.preferred_subtitle_language ?? '';
	const mode = user?.preferred_subtitle_mode ?? '';
	if (!audio && !subtitle && !mode) {
		const one = user?.preferred_language ?? '';
		return { audio: one, subtitle: one, mode: 'auto' };
	}
	return { audio, subtitle, mode: isMode(mode) ? mode : 'auto' };
}

/** Nothing to apply: the player's own defaults stand. */
export function noPreference(p: TrackPreferences): boolean {
	return !p.audio && !p.subtitle && p.mode === 'auto';
}

/**
 * The stream a player's "Default" audio choice names. A transcode start
 * without `audio_stream` plays the account's preferred language (D-280),
 * so an account with one names the file's default track (flagged
 * default, else first) to get it; without one, nothing is sent and the
 * server keeps the default by itself.
 */
export function defaultAudioStream(streams: MediaStream[], prefs: TrackPreferences): number | undefined {
	if (!prefs.audio) return undefined;
	const audios = streams.filter((s) => s.type === 'audio');
	return (audios.find((s) => s.default) ?? audios[0])?.index;
}

const untagged = (l: string | undefined) => !l || l === 'und';

/**
 * The subtitle the rule picks from `subs`, 'off', or 'keep' (the player's
 * default). `tagged`: some audio or subtitle track says its language.
 */
export function chooseSubtitle<T extends SubtitleCandidate>(
	subs: T[],
	prefs: TrackPreferences,
	audioLanguage: string,
	tagged: boolean
): T | 'off' | 'keep' {
	const lang = prefs.subtitle;
	const inLanguage = (l: string) =>
		subs.find((s) => !s.forced && languageMatches(s.language, l)) ??
		subs.find((s) => languageMatches(s.language, l));
	switch (prefs.mode) {
		case 'off':
			return 'off';
		case 'forced': {
			const forced = subs.filter((s) => s.forced);
			const by = (l: string) => (l ? forced.find((s) => languageMatches(s.language, l)) : undefined);
			return by(lang) ?? by(audioLanguage) ?? forced.find((s) => untagged(s.language)) ?? 'off';
		}
		case 'always': {
			if (!lang) {
				const full = subs.filter((s) => !s.forced);
				return full.find((s) => s.default) ?? full[0] ?? 'off';
			}
			return inLanguage(lang) ?? (tagged ? 'off' : 'keep');
		}
		default: {
			if (!lang || !tagged) return 'keep';
			if (languageMatches(audioLanguage, lang)) return 'off';
			return inLanguage(lang) ?? 'off';
		}
	}
}

interface Candidate extends SubtitleCandidate {
	embedded?: number;
	sidecar?: number;
}

/**
 * Applies the preferences (and this title's remembered choice) to a plan's
 * streams and the item's sidecar files, which share the subtitle choice:
 * embedded tracks first, so a tie keeps the file's own track.
 */
export function resolveTracks(
	streams: MediaStream[],
	prefs: TrackPreferences,
	remembered: RememberedTracks = {},
	sidecars: SidecarCandidate[] = []
): TrackChoice {
	const metadataKnown = streams.some(
		(s) => (s.type === 'audio' || s.type === 'subtitle') && !untagged(s.language)
	);
	const audios = streams.filter((s) => s.type === 'audio');
	const want = remembered.audio || prefs.audio;
	const matching = want ? audios.find((s) => languageMatches(s.language, want)) : undefined;
	const audio = matching ?? audios.find((s) => s.default) ?? audios[0];
	const choice: TrackChoice = { audioMatches: !!matching, metadataKnown };
	// Nothing asked and nothing remembered: the player's defaults stand.
	if (noPreference(prefs) && remembered.audio === undefined && remembered.subtitle === undefined) return choice;
	if (metadataKnown) choice.audioIndex = audio?.index;
	const subs: Candidate[] = [
		...streams
			.filter((s) => s.type === 'subtitle' && s.convertible)
			.map((s) => ({ language: s.language, forced: s.forced, default: s.default, embedded: s.index })),
		...sidecars.map((s) => ({ language: s.language, forced: s.forced, sidecar: s.index }))
	];
	const take = (c: Candidate): TrackChoice => ({ ...choice, subtitleIndex: c.embedded, sidecarIndex: c.sidecar });
	if (remembered.subtitle !== undefined) {
		if (remembered.subtitle === '') return choice;
		const lang = remembered.subtitle;
		const kept =
			subs.find((s) => !s.forced && languageMatches(s.language, lang)) ??
			subs.find((s) => languageMatches(s.language, lang));
		if (kept) return take(kept);
	}
	const tagged = metadataKnown || sidecars.some((s) => !untagged(s.language));
	const sub = chooseSubtitle(subs, prefs, audio?.language ?? '', tagged);
	return typeof sub === 'object' ? take(sub) : choice;
}

function languageName(code: string, locale: string): string {
	try {
		const name = new Intl.DisplayNames([locale], { type: 'language' }).of(code);
		return name && name !== code && name !== 'root' ? name : '';
	} catch {
		return '';
	}
}

function nativeName(code: string): string {
	try {
		return new Intl.DisplayNames([code], { type: 'language' }).of(code) ?? '';
	} catch {
		return '';
	}
}

/**
 * How a track menu names a track: its language in the interface's words
 * ("Portuguese" for `por`), with the file's own title when that says more
 * ("English · Commentary"), else the title alone, else `fallback`. The bare
 * tag ("por") read as a code, not a choice.
 */
export function trackName(
	track: { language?: string; title?: string },
	locale: string,
	fallback: string
): string {
	const code = (track.language ?? '').trim();
	const language = code && code !== 'und' ? languageName(code, locale) : '';
	const title = (track.title ?? '').trim();
	if (!language) return title || code || fallback;
	// A title that only names the language again, in either tongue, adds nothing.
	const same = (a: string) => a !== '' && a.toLocaleLowerCase() === title.toLocaleLowerCase();
	if (!title || same(language) || same(nativeName(code)) || same(code)) return language;
	return `${language} · ${title}`;
}
