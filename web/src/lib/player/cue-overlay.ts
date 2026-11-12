/*
 * Cue overlay geometry (D-082). While the GPU renderer presents frames the
 * browser subtitle track runs in `hidden` mode — it still parses cues and
 * fires cuechange, but paints nothing — and the player places the active
 * cues itself inside the letterbox. These helpers translate WebVTT cue
 * placement onto the letterboxed picture rectangle.
 */

export interface Rect {
	left: number;
	top: number;
	width: number;
	height: number;
}

// object-contain picture rect: the region of the element's box the decoded
// frame actually occupies, which is where native cues would anchor.
export function pictureRect(
	videoWidth: number,
	videoHeight: number,
	boxWidth: number,
	boxHeight: number
): Rect | null {
	if (videoWidth <= 0 || videoHeight <= 0 || boxWidth <= 0 || boxHeight <= 0) {
		return null;
	}
	const scale = Math.min(boxWidth / videoWidth, boxHeight / videoHeight);
	const width = videoWidth * scale;
	const height = videoHeight * scale;
	return { left: (boxWidth - width) / 2, top: (boxHeight - height) / 2, width, height };
}

export interface CuePlacement {
	left: number;
	top: number;
	width: number;
	textAlign: 'left' | 'center' | 'right';
}

function num(value: number | string | null | undefined, fallback: number): number {
	return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function horizontalAnchor(cue: VTTCue): { anchor: number; textAlign: 'left' | 'center' | 'right' } {
	switch (cue.align) {
		case 'start':
		case 'left':
			return { anchor: 0, textAlign: 'left' };
		case 'end':
		case 'right':
			return { anchor: 100, textAlign: 'right' };
		default:
			return { anchor: 50, textAlign: 'center' };
	}
}

// A cue with an explicit line is positioned on its own; `line: 'auto'` cues
// return null and join the bottom stack, mirroring how the browser treats
// unpositioned dialogue.
export function cuePlacement(cue: VTTCue, rect: Rect, lineHeight: number): CuePlacement | null {
	if (cue.line === 'auto' || cue.line === null || cue.line === undefined) return null;
	const { anchor, textAlign } = horizontalAnchor(cue);
	const position = Math.min(Math.max(num(cue.position, 50), 0), 100);
	const size = Math.min(Math.max(num(cue.size, 100), 0), 100);
	const width = (rect.width * size) / 100;
	const unclamped = rect.left + (position / 100) * rect.width - (anchor / 100) * width;
	const left = Math.min(Math.max(unclamped, rect.left), rect.left + rect.width - width);

	let top: number;
	if (cue.snapToLines) {
		const line = num(cue.line, 0);
		top = line >= 0 ? rect.top + line * lineHeight : rect.top + rect.height + line * lineHeight;
	} else {
		const line = Math.min(Math.max(num(cue.line, 100), 0), 100);
		top = rect.top + (line / 100) * rect.height;
	}
	return { left, top, width, textAlign };
}

/*
 * Whether the browser's own cue box can be moved. Chromium and Safari lay
 * cues out in ::-webkit-media-text-track-container, which takes a transform,
 * so the player lifts it above the controls while they show. Gecko has no
 * such box and ::cue cannot position anything, so there the player draws the
 * cues itself, with this overlay, where they can step above the controls.
 * (`CSS.supports` on the -webkit- pseudo-element cannot tell: unknown
 * -webkit- pseudo-elements parse everywhere, by spec.)
 */
export function nativeCueBoxMovable(): boolean {
	if (typeof CSS === 'undefined' || typeof CSS.supports !== 'function') return true;
	return !CSS.supports('-moz-appearance', 'none');
}

/*
 * Where the stack of unpositioned cues ends, as a distance from the box's
 * bottom edge: just inside the picture's lower edge, or just above the
 * controls while they cover that edge (`lift` is how far up they reach). A
 * letterboxed picture whose edge is already above the controls stays put.
 */
export function stackBottom(rect: Rect, boxHeight: number, lift: number): number {
	const resting = boxHeight - (rect.top + rect.height) + rect.height * 0.03;
	return lift > 0 ? Math.max(resting, lift + rect.height * 0.02) : resting;
}
