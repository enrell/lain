import { afterEach, describe, expect, it, vi } from 'vitest';
import {
	loadSkip,
	loadSubtitleBackground,
	loadSubtitleSize,
	loadTitleTracks,
	saveSkip,
	saveSubtitleBackground,
	saveSubtitleSize,
	saveTitleTrack
} from './player-prefs';

function memoryStorage(): Map<string, string> {
	const values = new Map<string, string>();
	vi.stubGlobal('localStorage', {
		getItem: (key: string) => values.get(key) ?? null,
		setItem: (key: string, value: string) => {
			values.set(key, value);
		}
	});
	return values;
}

describe('player prefs', () => {
	afterEach(() => vi.unstubAllGlobals());

	it('defaults to the automatic skip, a medium size and no box', () => {
		memoryStorage();
		expect(loadSkip()).toBe(0);
		expect(loadSubtitleSize()).toBe('medium');
		expect(loadSubtitleBackground()).toBe('none');
	});

	it('keeps the skip on the key the player has always used', () => {
		const values = memoryStorage();
		saveSkip(30);
		expect(values.get('lain.player.skip')).toBe('30');
		expect(loadSkip()).toBe(30);
	});

	it('ignores a stored value that is not one of the choices', () => {
		const values = memoryStorage();
		values.set('lain.player.skip', '7');
		values.set('lain.player.subtitle_size', 'huge');
		values.set('lain.player.subtitle_bg', 'blur');
		expect(loadSkip()).toBe(0);
		expect(loadSubtitleSize()).toBe('medium');
		expect(loadSubtitleBackground()).toBe('none');
	});

	it('round-trips the subtitle look', () => {
		memoryStorage();
		saveSubtitleSize('large');
		saveSubtitleBackground('box');
		expect(loadSubtitleSize()).toBe('large');
		expect(loadSubtitleBackground()).toBe('box');
	});

	it('survives storage that throws', () => {
		vi.stubGlobal('localStorage', {
			getItem: () => null,
			setItem: () => {
				throw new Error('quota');
			}
		});
		expect(() => saveSkip(10)).not.toThrow();
		expect(loadSkip()).toBe(0);
	});

	it('remembers the languages picked for a title, subtitles off included', () => {
		const values = memoryStorage();
		expect(loadTitleTracks('Frieren')).toEqual({});
		saveTitleTrack('Frieren', 'audio', 'JPN');
		saveTitleTrack('Frieren', 'subtitle', 'por');
		expect(loadTitleTracks(' frieren ')).toEqual({ audio: 'jpn', subtitle: 'por' });
		saveTitleTrack('Frieren', 'subtitle', null);
		expect(loadTitleTracks('Frieren')).toEqual({ audio: 'jpn', subtitle: '' });
		expect(values.get('lain.player.tracks.subtitle.frieren')).toBe('');
		// An untagged track is not a language to keep.
		saveTitleTrack('Frieren', 'subtitle', 'und');
		saveTitleTrack('Frieren', 'audio', undefined);
		expect(loadTitleTracks('Frieren')).toEqual({ audio: 'jpn', subtitle: '' });
		expect(loadTitleTracks('Other')).toEqual({});
	});
});
