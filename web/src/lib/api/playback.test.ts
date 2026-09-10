import { describe, expect, it } from 'vitest';
import { mediaUrl } from './client';
import { playback } from './playback';

describe('mediaUrl', () => {
	it('appends the token URL-encoded and only for media', () => {
		const url = mediaUrl('/api/items/abc/stream', 'tok en/+', { start: '1' });
		expect(url.startsWith('/api/items/abc/stream?')).toBe(true);
		const q = new URLSearchParams(url.split('?')[1]);
		expect(q.get('token')).toBe('tok en/+');
		expect(q.get('start')).toBe('1');
	});

	it('omits the token when there is none', () => {
		expect(mediaUrl('/api/items/abc/stream', null)).toBe('/api/items/abc/stream');
	});
});

describe('playback.streamUrl', () => {
	it('builds the gateway stream URL with an encoded item id', () => {
		const url = playback.streamUrl('id/with space', 'a.b.c');
		expect(url).toBe('/api/items/id%2Fwith%20space/stream?token=a.b.c');
	});

	it('never appends a token to ordinary API paths', () => {
		expect(playback.streamUrl('abc', null)).toBe('/api/items/abc/stream');
		expect(playback.streamUrl('abc', null)).not.toContain('token');
	});
});
