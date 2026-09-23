import type { MediaStream } from '$lib/api/types';

export interface TrackChoice {
	audioIndex?: number;
	subtitleIndex?: number;
	audioMatches: boolean;
	metadataKnown: boolean;
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
	return !!want && (equivalent[code] ?? code) === (equivalent[want] ?? want);
}

/** Prefer audio in the account language; otherwise show matching subtitles. */
export function resolveTracks(streams: MediaStream[], language: string): TrackChoice {
	if (!language) return { audioMatches: false, metadataKnown: false };
	const metadataKnown = streams.some((s) =>
		(s.type === 'audio' || s.type === 'subtitle') && !!s.language
	);
	if (!metadataKnown) return { audioMatches: false, metadataKnown: false };
	const audios = streams.filter((s) => s.type === 'audio');
	const matchingAudio = audios.find((s) => languageMatches(s.language, language));
	const audio = matchingAudio ?? audios.find((s) => s.default) ?? audios[0];
	const audioMatches = !!audio && languageMatches(audio.language, language);
	const subtitle = audioMatches ? undefined : streams.find((s) =>
		s.type === 'subtitle' && s.convertible && languageMatches(s.language, language)
	);
	return {
		audioIndex: audio?.index,
		subtitleIndex: subtitle?.index,
		audioMatches,
		metadataKnown
	};
}
