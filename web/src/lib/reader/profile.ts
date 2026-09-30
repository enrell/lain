import type { Direction } from './layout';

export type ReaderMode = 'paged' | 'strip';
export type ReaderFit = 'contain' | 'width';

/**
 * What differs between reading manga and reading comics. The base reader
 * renders whatever a profile enables; the profile never changes how a
 * page is fetched or how progress is saved.
 */
export interface ReaderProfile {
	kind: 'manga' | 'comic';
	/** Direction when the title has no saved choice; the server sends the kind's default. */
	direction: Direction;
	mode: ReaderMode;
	dual: boolean;
	coverAlone: boolean;
	fit: ReaderFit;
	/** Which controls the adaptation exposes. */
	controls: {
		direction: boolean;
		dual: boolean;
		coverAlone: boolean;
		strip: boolean;
		fit: boolean;
		zoom: boolean;
	};
	/** Word for the vertical continuous mode in this kind's vocabulary. */
	stripLabel: string;
}

/**
 * Manga: right-to-left, two-page spreads on wide screens with the cover
 * alone, single tap zones that follow the direction, and a webtoon-style
 * strip for long-scroll titles.
 */
export const MANGA_PROFILE: ReaderProfile = {
	kind: 'manga',
	direction: 'rtl',
	mode: 'paged',
	dual: true,
	coverAlone: true,
	fit: 'contain',
	controls: { direction: true, dual: true, coverAlone: true, strip: true, fit: false, zoom: false },
	stripLabel: 'Webtoon'
};

/**
 * Comics: left-to-right, one full page at a time by default (western
 * pages are dense and read best large), with fit and zoom controls for
 * detail and a continuous scroll for digital-first titles.
 */
export const COMIC_PROFILE: ReaderProfile = {
	kind: 'comic',
	direction: 'ltr',
	mode: 'paged',
	dual: false,
	coverAlone: true,
	fit: 'contain',
	controls: { direction: false, dual: true, coverAlone: false, strip: true, fit: true, zoom: true },
	stripLabel: 'Scroll'
};
