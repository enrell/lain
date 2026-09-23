import { describe, expect, it } from 'vitest';
import { DecodedFrameProbe } from './frame-probe';

const video = {} as HTMLVideoElement;

describe('decoded frame probe', () => {
	it('keeps native video visible until a readable decoded frame arrives', () => {
		let visible = false;
		let time = 0;
		const probe = new DecodedFrameProbe(() => visible, () => time);
		expect(probe.check(video)).toBe(false);
		time = 4000;
		visible = true;
		expect(probe.check(video)).toBe(true);
	});

	it('reports a browser that returns blank frames continuously', () => {
		let time = 0;
		const probe = new DecodedFrameProbe(() => false, () => time);
		expect(probe.check(video)).toBe(false);
		time = 5000;
		expect(() => probe.check(video)).toThrow('browser returns blank decoded video frames');
	});

	it('rechecks active playback and lets the native picture take over on blank frames', () => {
		let visible = true;
		const probe = new DecodedFrameProbe(() => visible, () => 0);
		expect(probe.check(video)).toBe(true);
		visible = false;
		for (let frame = 2; frame < 30; frame++) expect(probe.check(video)).toBe(true);
		expect(probe.check(video)).toBe(false);
		visible = true;
		expect(probe.check(video)).toBe(true);
	});

	it('propagates frame inspection failures without replacing native video', () => {
		const probe = new DecodedFrameProbe(() => { throw new Error('readback failed'); });
		expect(() => probe.check(video)).toThrow('browser cannot inspect decoded video frames: readback failed');
	});
});
