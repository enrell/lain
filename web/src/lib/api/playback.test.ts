import { afterEach, describe, expect, it, vi } from 'vitest';
import { mediaUrl } from './client';
import { playback } from './playback';

afterEach(() => vi.unstubAllGlobals());

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

describe('playback.transcodeUrl', () => {
	it('builds the gateway transcode URL with an encoded item id', () => {
		const url = playback.transcodeUrl('id/with space', 'a.b.c', 'session-1');
		expect(url).toContain('/api/items/id%2Fwith%20space/transcode?');
		const query = new URLSearchParams(url.split('?')[1]);
		expect(query.get('token')).toBe('a.b.c');
		expect(query.get('session')).toBe('session-1');
	});

	it('omits the token when there is none', () => {
		expect(playback.transcodeUrl('abc', null)).toBe('/api/items/abc/transcode');
		expect(playback.transcodeUrl('abc', null)).not.toContain('token');
	});
});

describe('playback async transcode', () => {
	it('starts and polls an item-scoped session without putting tokens in the URL', async () => {
		const fetchMock = vi.fn()
			.mockResolvedValueOnce(new Response(JSON.stringify({ session: 's1', state: 'queued', profile: 'p1' }), { status: 202 }))
			.mockResolvedValueOnce(new Response(JSON.stringify({ session: 's1', state: 'ready', profile: 'p1' }), { status: 200 }));
		vi.stubGlobal('fetch', fetchMock);

		await playback.startTranscode('id/one');
		await playback.transcodeStatus('id/one', 's1');

		expect(fetchMock.mock.calls[0][0]).toBe('/api/items/id%2Fone/transcode');
		expect(fetchMock.mock.calls[0][1].method).toBe('POST');
		expect(fetchMock.mock.calls[1][0]).toBe('/api/items/id%2Fone/transcode/status?session=s1');
		expect(fetchMock.mock.calls[1][0]).not.toContain('token');
	});
});
