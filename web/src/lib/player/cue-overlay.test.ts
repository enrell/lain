import { describe, expect, it } from 'vitest';
import { cuePlacement, pictureRect, type Rect } from './cue-overlay';

const BOX: Rect = { left: 0, top: 0, width: 800, height: 450 };

function cue(overrides: Partial<VTTCue>): VTTCue {
	return {
		align: 'center',
		line: 'auto',
		position: 'auto',
		size: 100,
		snapToLines: false,
		...overrides
	} as VTTCue;
}

describe('pictureRect', () => {
	it('letterboxes a 16:9 picture inside a wider element', () => {
		const rect = pictureRect(1920, 1080, 1000, 500);
		expect(rect?.top).toBe(0);
		expect(rect?.height).toBe(500);
		expect(rect?.width).toBeCloseTo(888.8889);
		expect(rect?.left).toBeCloseTo(55.5556);
	});

	it('pillarboxes a tall element', () => {
		expect(pictureRect(1920, 1080, 800, 1000)).toEqual({
			left: 0,
			top: (1000 - 800 / (1920 / 1080)) / 2,
			width: 800,
			height: 800 / (1920 / 1080)
		});
	});

	it('returns null before metadata loads', () => {
		expect(pictureRect(0, 0, 800, 450)).toBeNull();
	});
});

describe('cuePlacement', () => {
	it('returns null for auto lines so the cue joins the bottom stack', () => {
		expect(cuePlacement(cue({ line: 'auto' }), BOX, 20)).toBeNull();
	});

	it('anchors a percentage line cue at the picture edge', () => {
		const p = cuePlacement(cue({ line: 80, snapToLines: false }), BOX, 20);
		expect(p).toEqual({ left: 0, top: 80 / 100 * 450, width: 800, textAlign: 'center' });
	});

	it('honors position, size and end alignment', () => {
		const p = cuePlacement(cue({ line: 10, position: 90, size: 40, align: 'end' }), BOX, 20);
		// anchor edge at 90% of 800 = 720; end-aligned box of 320 ends there.
		expect(p?.left).toBeCloseTo(720 - 320);
		expect(p?.width).toBeCloseTo(320);
		expect(p?.textAlign).toBe('right');
	});

	it('clamps the cue box inside the picture', () => {
		const p = cuePlacement(cue({ line: 50, position: 10, size: 60, align: 'end' }), BOX, 20);
		expect(p?.left).toBe(0);
	});

	it('counts snapped lines from the top for positive lines', () => {
		const p = cuePlacement(cue({ line: 3, snapToLines: true }), BOX, 20);
		expect(p?.top).toBeCloseTo(3 * 20);
	});

	it('counts snapped lines from the bottom for negative lines', () => {
		const p = cuePlacement(cue({ line: -2, snapToLines: true }), BOX, 20);
		expect(p?.top).toBeCloseTo(450 - 2 * 20);
	});
});
