/*
 * The decode capability probe (D-058).
 *
 * The plan request is made before the client knows the file's streams, so
 * the client cannot answer per file. It answers per *vocabulary* instead:
 * it probes a fixed matrix once and reports the tokens it could verify,
 * which the server validates against its own whitelist and applies to the
 * probed streams. A token is either a container ("mkv": the container
 * opens) or a container/codec pair ("mkv/h264": that codec family decodes
 * inside that container) — the pairing is what stops "HEVC decodes in
 * MP4" from licensing "HEVC in Matroska", which is a black screen.
 *
 * Two a-priori queries disagree, each in a different direction (measured
 * on Chrome 152): `decodingInfo` is right about HEVC and AC-3 but returns
 * a false negative for VP9+Opus, which in fact plays; `canPlayType` with
 * a codec string cannot see a platform decoder it has no string for. So a
 * token is reported when *either* says yes. The cost of a false positive
 * is one verification round-trip — the player proves the first decoded
 * frame and falls back to a transcode — while the cost of a false
 * negative is re-encoding a file the browser could have played.
 *
 * The matrix must stay in step with internal/contracts/capabilities.go:
 * a token the server does not know is dropped there, so drift costs
 * direct play silently. The browser smoke test is the guard.
 */

/** One codec the client tries to prove, with its RFC 6381 string. */
interface CodecProbe {
	token: string;
	/** null when the container's codec takes no codecs parameter. */
	codec: string | null;
}

interface ContainerProbe {
	token: string;
	/** Empty when the container carries no video (mp3, flac). */
	videoMime: string;
	audioMime: string;
	videoCodecs: CodecProbe[];
	audioCodecs: CodecProbe[];
}

const VIDEO_CODECS: Record<string, CodecProbe> = {
	h264: { token: 'h264', codec: 'avc1.64001f' },
	hevc: { token: 'hevc', codec: 'hvc1.1.6.L93.B0' },
	vp8: { token: 'vp8', codec: 'vp8' },
	vp9: { token: 'vp9', codec: 'vp09.00.10.08' },
	av1: { token: 'av1', codec: 'av01.0.04M.08' }
};

const AUDIO_CODECS: Record<string, CodecProbe> = {
	aac: { token: 'aac', codec: 'mp4a.40.2' },
	mp3: { token: 'mp3', codec: 'mp3' },
	opus: { token: 'opus', codec: 'opus' },
	vorbis: { token: 'vorbis', codec: 'vorbis' },
	flac: { token: 'flac', codec: 'flac' },
	ac3: { token: 'ac3', codec: 'ac-3' },
	eac3: { token: 'eac3', codec: 'ec-3' }
};

const MATRIX: ContainerProbe[] = [
	{
		token: 'mp4',
		videoMime: 'video/mp4',
		audioMime: 'audio/mp4',
		videoCodecs: [VIDEO_CODECS.h264, VIDEO_CODECS.hevc, VIDEO_CODECS.av1, VIDEO_CODECS.vp9],
		audioCodecs: [
			AUDIO_CODECS.aac,
			AUDIO_CODECS.mp3,
			AUDIO_CODECS.ac3,
			AUDIO_CODECS.eac3,
			AUDIO_CODECS.opus,
			AUDIO_CODECS.flac
		]
	},
	{
		token: 'mkv',
		videoMime: 'video/x-matroska',
		audioMime: 'audio/x-matroska',
		videoCodecs: [VIDEO_CODECS.h264, VIDEO_CODECS.hevc, VIDEO_CODECS.vp9, VIDEO_CODECS.av1],
		audioCodecs: [
			AUDIO_CODECS.aac,
			AUDIO_CODECS.mp3,
			AUDIO_CODECS.opus,
			AUDIO_CODECS.vorbis,
			AUDIO_CODECS.flac,
			AUDIO_CODECS.ac3,
			AUDIO_CODECS.eac3
		]
	},
	{
		token: 'webm',
		videoMime: 'video/webm',
		audioMime: 'audio/webm',
		videoCodecs: [VIDEO_CODECS.vp8, VIDEO_CODECS.vp9, VIDEO_CODECS.av1],
		audioCodecs: [AUDIO_CODECS.opus, AUDIO_CODECS.vorbis]
	},
	{
		token: 'ogg',
		videoMime: 'video/ogg',
		audioMime: 'audio/ogg',
		videoCodecs: [],
		audioCodecs: [AUDIO_CODECS.opus, AUDIO_CODECS.vorbis, AUDIO_CODECS.flac]
	},
	{
		token: 'mp3',
		videoMime: '',
		audioMime: 'audio/mpeg',
		videoCodecs: [],
		audioCodecs: [{ token: 'mp3', codec: null }]
	},
	{
		token: 'flac',
		videoMime: '',
		audioMime: 'audio/flac',
		videoCodecs: [],
		audioCodecs: [{ token: 'flac', codec: null }]
	}
];

/** The two questions a browser can answer about one configuration. */
export interface CapabilityOracles {
	/** `canPlayType`, possibly with a codecs parameter. */
	canPlay: (type: string) => string;
	/** `navigator.mediaCapabilities.decodingInfo().supported`. */
	decodingSupported: (mime: string, codec: string | null) => Promise<boolean>;
}

/*
 * The pure half of the probe: given the two oracles a browser offers,
 * which tokens does this client report? Exported so the matrix and the
 * union rule are testable without a browser.
 */
export async function tokensFor(oracles: CapabilityOracles): Promise<string[]> {
	const tokens: string[] = [];
	for (const container of MATRIX) {
		const opens = [container.videoMime, container.audioMime].some(
			(mime) => mime !== '' && oracles.canPlay(mime) !== ''
		);
		if (!opens) continue;
		tokens.push(container.token);
		for (const side of [
			{ mime: container.videoMime, codecs: container.videoCodecs },
			{ mime: container.audioMime, codecs: container.audioCodecs }
		]) {
			if (side.mime === '') continue;
			for (const codec of side.codecs) {
				const type = codec.codec === null ? side.mime : `${side.mime}; codecs="${codec.codec}"`;
				if (oracles.canPlay(type) !== '' || (await oracles.decodingSupported(side.mime, codec.codec))) {
					tokens.push(`${container.token}/${codec.token}`);
				}
			}
		}
	}
	return tokens;
}

let probed: Promise<string> | null = null;

/*
 * The capability list to send on a plan request, probed once per page load
 * and cached: the matrix is fixed and a browser's decoders do not change
 * under it mid-session. An empty list is a valid answer — the client
 * claims nothing and the server keeps its conservative rules — so an
 * environment without a document (a test runner) probes nothing.
 */
export function reportCapabilities(): Promise<string> {
	probed ??= probe();
	return probed;
}

async function probe(): Promise<string> {
	if (typeof document === 'undefined' || typeof navigator === 'undefined') return '';
	const media = document.createElement('video');
	const tokens = await tokensFor({
		canPlay: (type) => media.canPlayType(type),
		decodingSupported: decodingSupported
	});
	return tokens.join(',');
}

async function decodingSupported(mime: string, codec: string | null): Promise<boolean> {
	const capabilities = navigator.mediaCapabilities as MediaCapabilities | undefined;
	if (!capabilities || typeof capabilities.decodingInfo !== 'function') return false;
	const contentType = codec === null ? mime : `${mime}; codecs="${codec}"`;
	const config: MediaDecodingConfiguration = mime.startsWith('video/')
		? {
				type: 'file',
				video: { contentType, width: 1920, height: 1080, bitrate: 8_000_000, framerate: 24 }
			}
		: { type: 'file', audio: { contentType, channels: '2', bitrate: 128_000, samplerate: 48_000 } };
	try {
		const info = await capabilities.decodingInfo(config);
		return info.supported;
	} catch {
		// A configuration the implementation refuses to parse is a "no",
		// not a crash: the whole list is an optimisation, never a
		// prerequisite for playback.
		return false;
	}
}
