import { describe, expect, it } from 'vitest';
import { VideoFrameScheduler, type VideoFrameSource } from './scheduler';

class FakeVideo implements VideoFrameSource {
	private nextId = 1;
	private callbacks = new Map<number, VideoFrameRequestCallback>();
	cancelled: number[] = [];

	requestVideoFrameCallback(callback: VideoFrameRequestCallback): number {
		const id = this.nextId++;
		this.callbacks.set(id, callback);
		return id;
	}

	cancelVideoFrameCallback(id: number): void {
		this.cancelled.push(id);
		this.callbacks.delete(id);
	}

	deliver(now = 10, mediaTime = 1): void {
		const entry = this.callbacks.entries().next().value as
			| [number, VideoFrameRequestCallback]
			| undefined;
		if (!entry) throw new Error('no frame callback');
		this.callbacks.delete(entry[0]);
		entry[1](now, { mediaTime } as VideoFrameCallbackMetadata);
	}

	pending(): number {
		return this.callbacks.size;
	}
}

describe('VideoFrameScheduler', () => {
	it('renders once for each decoded video frame and keeps one callback pending', () => {
		const video = new FakeVideo();
		const frames: number[] = [];
		const scheduler = new VideoFrameScheduler(video, (_now, metadata) => {
			frames.push(metadata.mediaTime);
		});

		scheduler.start();
		expect(video.pending()).toBe(1);
		video.deliver(10, 1);
		expect(frames).toEqual([1]);
		expect(video.pending()).toBe(1);
		video.deliver(20, 2);
		expect(frames).toEqual([1, 2]);
		expect(video.pending()).toBe(1);
	});

	it('cancels the pending callback and ignores delivery after stop', () => {
		const video = new FakeVideo();
		let frames = 0;
		const scheduler = new VideoFrameScheduler(video, () => frames++);

		scheduler.start();
		scheduler.stop();
		expect(video.cancelled).toEqual([1]);
		expect(video.pending()).toBe(0);
		expect(frames).toBe(0);
	});

	it('does not register duplicate loops', () => {
		const video = new FakeVideo();
		const scheduler = new VideoFrameScheduler(video, () => undefined);
		scheduler.start();
		scheduler.start();
		expect(video.pending()).toBe(1);
	});
});
