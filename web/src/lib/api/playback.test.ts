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

describe('playback.plan', () => {
	it('reports what this browser proved and lets the server decide', async () => {
		// A browser whose Matroska container opens but whose codecs prove
		// nothing: the request must carry exactly that claim — the
		// container, no codec pairs — and the answer stays the server's
		// (a container alone is not a licence to direct-play, D-058).
		vi.stubGlobal('document', {
			createElement: () => ({
				canPlayType: (type: string) => (type === 'video/x-matroska' ? 'maybe' : '')
			})
		});
		vi.stubGlobal('navigator', {
			mediaCapabilities: { decodingInfo: async () => ({ supported: false }) }
		});
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(new Response(JSON.stringify({ mode: 'transcode' }), { status: 200 }));
		vi.stubGlobal('fetch', fetchMock);

		const plan = await playback.plan('id/one');

		const url = fetchMock.mock.calls[0][0] as string;
		const query = new URLSearchParams(url.split('?')[1]);
		expect(url.startsWith('/api/items/id%2Fone/playback?')).toBe(true);
		expect(query.get('client')).toBe('web');
		expect(query.get('caps')).toBe('mkv');
		expect(plan.mode).toBe('transcode');
	});
});
