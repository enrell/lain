import { describe, expect, it } from 'vitest';
import { externalPlayerUrl } from './external-player';

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
