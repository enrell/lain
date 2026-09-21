export interface VideoFrameSource {
	requestVideoFrameCallback(callback: VideoFrameRequestCallback): number;
	cancelVideoFrameCallback(id: number): void;
}

/** Schedules exactly one GPU submission for each frame sent to the compositor. */
export class VideoFrameScheduler {
	private callbackID: number | null = null;
	private running = false;

	constructor(
		private readonly video: VideoFrameSource,
		private readonly render: VideoFrameRequestCallback
	) {}

	start(): void {
		if (this.running) return;
		this.running = true;
		this.schedule();
	}

	stop(): void {
		this.running = false;
		if (this.callbackID !== null) {
			this.video.cancelVideoFrameCallback(this.callbackID);
			this.callbackID = null;
		}
	}

	private schedule(): void {
		this.callbackID = this.video.requestVideoFrameCallback((now, metadata) => {
			this.callbackID = null;
			if (!this.running) return;
			// Register first so a synchronous render failure cannot silently end
			// the decoded-frame loop. The renderer owns failure reporting.
			this.schedule();
			this.render(now, metadata);
		});
	}
}
