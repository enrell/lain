import { describe, expect, it } from 'vitest';
import { externalPlayerUrl, playbackHref } from './external-player';

describe('externalPlayerUrl', () => {
	it('sends the origin and item ID without a credential', () => {
		const uri = externalPlayerUrl('https://media.example', 'id with space', 'vlc');
		const url = new URL(uri);
		expect(url.protocol).toBe('lain:');
		expect(url.hostname).toBe('play');
		expect(url.searchParams.get('server')).toBe('https://media.example');
		expect(url.searchParams.get('id')).toBe('id with space');
		expect(url.searchParams.get('player')).toBe('vlc');
		expect([...url.searchParams.keys()]).toEqual(['server', 'id', 'player']);
	});
});

describe('playbackHref', () => {
	it('keeps "local" on the web player href so bare navigation still works', () => {
		expect(playbackHref('https://media.example', 'it1', 'local')).toBe('/player/it1');
		expect(playbackHref('https://media.example', 'it1', 'browser')).toBe('/player/it1');
	});
	it('hands external players the lain:// protocol link', () => {
		expect(playbackHref('https://media.example', 'it1', 'mpv')).toBe(
			externalPlayerUrl('https://media.example', 'it1', 'mpv')
		);
	});
});
