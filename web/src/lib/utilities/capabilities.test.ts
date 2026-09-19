import { describe, expect, it } from 'vitest';
import { tokensFor, type CapabilityOracles } from './capabilities';

/*
 * An oracle pair answering from fixed accept lists, so a test states
 * exactly which of the two questions said yes.
 */
function oracles(accepts: string[], decoding: string[] = []): CapabilityOracles {
	const canPlay = new Set(accepts);
	const decodes = new Set(decoding);
	return {
		canPlay: (type) => (canPlay.has(type) ? 'probably' : ''),
		decodingSupported: async (mime, codec) =>
			decodes.has(codec === null ? mime : `${mime}; codecs="${codec}"`)
	};
}

describe('tokensFor', () => {
	it('reports nothing when the browser can play nothing', async () => {
		expect(await tokensFor(oracles([]))).toEqual([]);
	});

	it('reports a codec only inside a container that opens', async () => {
		// A client whose MP4 opens and whose H.264/AAC decode — the shape
		// of a Chrome that can play the catalog's MP4s. Nothing else may
		// be claimed: no container it never opened, no codec it never
		// proved.
		const tokens = await tokensFor(
			oracles(['video/mp4', 'video/mp4; codecs="avc1.64001f"', 'audio/mp4; codecs="mp4a.40.2"'])
		);
		expect(tokens).toContain('mp4');
		expect(tokens).toContain('mp4/h264');
		expect(tokens).toContain('mp4/aac');
		expect(tokens).not.toContain('mkv');
		expect(tokens).not.toContain('mp4/hevc');
		expect(tokens).not.toContain('mp4/ac3');
	});

	it('keeps a codec only decodingInfo can see', async () => {
		// The measured failure mode of `canPlayType` for a codec inside a
		// container it has no string for (VP9 in Matroska): the container
		// opens, the codec query is empty, and the union is what keeps the
		// file out of a needless re-encode.
		const tokens = await tokensFor(
			oracles(['video/x-matroska'], ['video/x-matroska; codecs="vp09.00.10.08"'])
		);
		expect(tokens).toEqual(['mkv', 'mkv/vp9']);
	});

	it('never reports a codec for a container that did not open', async () => {
		// decodingInfo is happy to say "supported" for a configuration the
		// browser cannot demux; the container question gates the answer.
		const tokens = await tokensFor(
			oracles([], ['video/x-matroska; codecs="avc1.64001f"', 'audio/x-matroska; codecs="mp4a.40.2"'])
		);
		expect(tokens).toEqual([]);
	});

	it('reports single-codec containers without a codecs parameter', async () => {
		const tokens = await tokensFor(oracles(['audio/mpeg', 'audio/flac']));
		expect(tokens).toEqual(['mp3', 'mp3/mp3', 'flac', 'flac/flac']);
	});

	it('separates the audio side of a container from its video side', async () => {
		// A browser that opens Matroska for video and decodes H.264 in it,
		// but proves nothing about its audio, must not claim mkv/aac —
		// that is the claim that licenses a silent or black playback.
		const tokens = await tokensFor(oracles(['video/x-matroska', 'video/x-matroska; codecs="avc1.64001f"']));
		expect(tokens).toEqual(['mkv', 'mkv/h264']);
	});
});
