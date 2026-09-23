const SAMPLE_WIDTH = 32;
const SAMPLE_HEIGHT = 18;
const BRIGHT_CHANNEL = 8;
const MIN_VISIBLE_PIXELS = 4;
const BLANK_TIMEOUT_MS = 5000;

function sampleVideo(video: HTMLVideoElement): boolean {
	const canvas = document.createElement('canvas');
	canvas.width = SAMPLE_WIDTH;
	canvas.height = SAMPLE_HEIGHT;
	const context = canvas.getContext('2d', { willReadFrequently: true });
	if (!context) throw new Error('video frame inspection is unavailable');
	context.drawImage(video, 0, 0, SAMPLE_WIDTH, SAMPLE_HEIGHT);
	const pixels = context.getImageData(0, 0, SAMPLE_WIDTH, SAMPLE_HEIGHT).data;
	let visible = 0;
	for (let index = 0; index < pixels.length; index += 4) {
		if (Math.max(pixels[index], pixels[index + 1], pixels[index + 2]) > BRIGHT_CHANNEL &&
			++visible >= MIN_VISIBLE_PIXELS) return true;
	}
	return false;
}

/** Prevents an inaccessible decoded frame from replacing native video with black. */
export class DecodedFrameProbe {
	private checkedFrames = 0;
	private readable = false;
	private blankSince: number | null = null;

	constructor(
		private readonly sample: (video: HTMLVideoElement) => boolean = sampleVideo,
		private readonly now: () => number = () => performance.now()
	) {}

	check(video: HTMLVideoElement): boolean {
		this.checkedFrames++;
		if (this.readable && this.checkedFrames % 30 !== 0) return true;
		let visible: boolean;
		try {
			visible = this.sample(video);
		} catch (error) {
			throw new Error(`browser cannot inspect decoded video frames: ${error instanceof Error ? error.message : String(error)}`);
		}
		if (visible) {
			this.blankSince = null;
			this.readable = true;
			return true;
		}
		const now = this.now();
		this.blankSince ??= now;
		if (now - this.blankSince >= BLANK_TIMEOUT_MS) {
			throw new Error('browser returns blank decoded video frames; native playback is shown');
		}
		return false;
	}
}
