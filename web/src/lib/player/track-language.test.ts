import { describe, expect, it } from 'vitest';
import type { MediaStream, User } from '$lib/api/types';
import {
	defaultAudioStream,
	resolveTracks,
	trackName,
	trackPreferences,
	type RememberedTracks,
	type SidecarCandidate,
	type TrackPreferences
} from './track-language';

// The cases of lain-watch's tracks.rs (D-280), on the web's stream shape.
const p = (audio: string, subtitle: string, mode: TrackPreferences['mode'] = 'auto'): TrackPreferences => ({
	audio,
	subtitle,
	mode
});
const sub = (index: number, language: string, extra: Partial<MediaStream> = {}): MediaStream => ({
	type: 'subtitle', index, codec: 'ass', language, convertible: true, ...extra
});

/** English dub first and default, Japanese second, signs, full Portuguese and English. */
const anime: MediaStream[] = [
	{ type: 'video', index: 0, codec: 'h264' },
	{ type: 'audio', index: 1, codec: 'aac', language: 'eng', default: true },
	{ type: 'audio', index: 2, codec: 'aac', language: 'jpn' },
	sub(3, 'eng', { forced: true }),
	sub(4, 'por'),
	sub(5, 'eng')
];
/** Dubbed in Portuguese, English subtitles only. */
const dubbed: MediaStream[] = [
	{ type: 'audio', index: 1, codec: 'aac', language: 'por' },
	sub(2, 'eng')
];
const pick = (
	streams: MediaStream[],
	prefs: TrackPreferences,
	remembered: RememberedTracks = {},
	files: SidecarCandidate[] = []
) => {
	const c = resolveTracks(streams, prefs, remembered, files);
	return [c.audioIndex, c.subtitleIndex ?? (c.sidecarIndex === undefined ? undefined : `sc:${c.sidecarIndex}`)];
};

describe('track preferences (D-280)', () => {
	it("the owner's case: Japanese audio, Portuguese subtitles, none over a dub", () => {
		expect(pick(anime, p('jpn', 'por'))).toEqual([2, 4]);
		expect(pick(dubbed, p('jpn', 'por'))).toEqual([1, undefined]);
		expect(pick(anime, p('jpn', 'eng'))).toEqual([2, 5]); // the full track, not the signs
	});

	it('one language is D-071', () => {
		const por = p('por', 'por');
		expect(pick(dubbed, por)).toEqual([1, undefined]);
		expect(pick([{ type: 'audio', index: 1, codec: 'aac', language: 'jpn' }, sub(2, 'por')], por)).toEqual([1, 2]);
		expect(resolveTracks([{ type: 'audio', index: 1, codec: 'aac' }], por)).toEqual({ audioMatches: false, metadataKnown: false });
		expect(resolveTracks(anime, p('', ''))).toEqual({ audioMatches: false, metadataKnown: true });
	});

	it('modes', () => {
		expect(pick(dubbed, p('', 'eng', 'always'))).toEqual([1, 2]);
		expect(pick(dubbed, p('', 'por', 'always'))).toEqual([1, undefined]);
		const flagged = [
			{ type: 'audio', index: 1, codec: 'aac', language: 'jpn' },
			sub(2, 'eng', { forced: true }),
			sub(3, 'eng'),
			sub(4, 'spa', { default: true })
		] satisfies MediaStream[];
		expect(pick(flagged, p('', '', 'always'))).toEqual([1, 4]);
		expect(pick(anime, p('', 'eng', 'forced'))).toEqual([1, 3]);
		expect(pick(anime, p('', '', 'forced'))).toEqual([1, 3]); // signs for the English audio
		expect(pick(anime, p('jpn', 'por', 'forced'))).toEqual([2, undefined]);
		expect(pick(anime, p('jpn', 'por', 'off'))).toEqual([2, undefined]);
	});

	it('accepts ISO 639 bibliographic aliases', () => {
		expect(pick([{ type: 'audio', index: 1, codec: 'aac', language: 'jpn' }, sub(2, 'fre')], p('', 'fra'))).toEqual([1, 2]);
	});

	it('weighs sidecar files with the embedded tracks', () => {
		const files = [
			{ index: 0, language: 'por', forced: true },
			{ index: 1, language: 'jpn' },
			{ index: 2, language: 'por' }
		];
		expect(pick(dubbed, p('jpn', 'eng'), {}, files)).toEqual([1, 2]); // embedded first on a tie
		const jp = [{ type: 'audio', index: 1, codec: 'aac', language: 'jpn' }] satisfies MediaStream[];
		expect(pick(jp, p('jpn', 'por'), {}, files)).toEqual([1, 'sc:2']); // the full file, not the forced one
		expect(pick(dubbed, p('por', 'por'), {}, files)).toEqual([1, undefined]); // the audio is the language
		expect(pick(jp, p('', ''), {}, files)).toEqual([undefined, undefined]);
	});

	it("this title's remembered choice wins over the account", () => {
		expect(pick(anime, p('jpn', 'por'), { audio: 'eng' })).toEqual([1, 4]);
		expect(pick(anime, p('jpn', 'por'), { subtitle: '' })).toEqual([2, undefined]);
		expect(pick(anime, p('jpn', 'por'), { subtitle: 'eng' })).toEqual([2, 5]);
		// Remembered on a title the account has no preference for.
		expect(pick(anime, p('', ''), { audio: 'jpn', subtitle: 'por' })).toEqual([2, 4]);
		// A language the file lacks falls back to the account's rule.
		expect(pick(anime, p('jpn', 'por'), { subtitle: 'kor' })).toEqual([2, 4]);
	});

	it("names the file's default audio for a Default pick only when the server would pick another", () => {
		expect(defaultAudioStream(anime, p('jpn', 'por'))).toBe(1);
		expect(defaultAudioStream(anime, p('', 'por'))).toBeUndefined();
		const unflagged = [
			{ type: 'audio', index: 3, codec: 'aac', language: 'jpn' },
			{ type: 'audio', index: 4, codec: 'aac', language: 'eng' }
		] satisfies MediaStream[];
		expect(defaultAudioStream(unflagged, p('eng', ''))).toBe(3);
		expect(defaultAudioStream([sub(1, 'eng')], p('jpn', ''))).toBeUndefined();
	});

	it('reads an account from before D-280 as its one language', () => {
		const user = (u: Partial<User>) => ({ id: 'u', username: 'u', role: 'user', disabled: false, pwd_ver: 1, created_at: 0, ...u }) as User;
		expect(trackPreferences(user({ preferred_language: 'jpn' }))).toEqual(p('jpn', 'jpn'));
		expect(trackPreferences(user({}))).toEqual(p('', ''));
		expect(trackPreferences(null)).toEqual(p('', ''));
		expect(
			trackPreferences(user({ preferred_language: 'por', preferred_audio_language: 'jpn', preferred_subtitle_language: 'por' }))
		).toEqual(p('jpn', 'por'));
		expect(trackPreferences(user({ preferred_language: 'jpn', preferred_audio_language: 'jpn', preferred_subtitle_mode: 'off' }))).toEqual(
			p('jpn', '', 'off')
		);
	});
});

describe('trackName', () => {
	it('names the language in the interface language, not by its tag', () => {
		expect(trackName({ language: 'por', title: 'Português' }, 'en', 'Track 4')).toBe('Portuguese');
		expect(trackName({ language: 'eng', title: 'English' }, 'en', 'Track 3')).toBe('English');
		expect(trackName({ language: 'jpn' }, 'en', 'Track 2')).toBe('Japanese');
	});

	it('keeps a title that says more than the language', () => {
		expect(trackName({ language: 'eng', title: 'Commentary' }, 'en', 'Track 3')).toBe('English · Commentary');
	});

	it('falls back to the title, the tag, then the fallback', () => {
		expect(trackName({ language: 'und', title: 'Signs' }, 'en', 'Track 1')).toBe('Signs');
		expect(trackName({ title: 'Director' }, 'en', 'Track 1')).toBe('Director');
		expect(trackName({ language: 'qaa' }, 'en', 'Track 1')).toBe('qaa');
		expect(trackName({}, 'en', 'Track 1')).toBe('Track 1');
	});
});
