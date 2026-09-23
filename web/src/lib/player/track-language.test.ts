import { describe, expect, it } from 'vitest';
import type { MediaStream } from '$lib/api/types';
import { resolveTracks } from './track-language';

const tracks: MediaStream[] = [
	{ type: 'audio', index: 1, codec: 'aac', language: 'jpn', default: true },
	{ type: 'audio', index: 2, codec: 'aac', language: 'por' },
	{ type: 'subtitle', index: 3, codec: 'ass', language: 'por', convertible: true },
	{ type: 'subtitle', index: 4, codec: 'ass', language: 'eng', convertible: true }
];

describe('preferred language track selection', () => {
	it('uses matching audio and disables subtitles', () => {
		expect(resolveTracks(tracks, 'por')).toEqual({ audioIndex: 2, subtitleIndex: undefined, audioMatches: true, metadataKnown: true });
	});
	it('uses matching subtitles when audio is unavailable', () => {
		expect(resolveTracks(tracks, 'eng')).toEqual({ audioIndex: 1, subtitleIndex: 4, audioMatches: false, metadataKnown: true });
	});
	it('does not invent a match for untagged tracks', () => {
		expect(resolveTracks([{ type: 'audio', index: 1, codec: 'aac' }], 'por')).toEqual({ audioMatches: false, metadataKnown: false });
	});
	it('accepts ISO 639 bibliographic aliases', () => {
		expect(resolveTracks([{ type: 'subtitle', index: 3, codec: 'ass', language: 'fre', convertible: true }], 'fra').subtitleIndex).toBe(3);
	});
});
