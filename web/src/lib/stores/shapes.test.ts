import { describe, expect, it } from 'vitest';
import type { CatalogItem, Enrichment } from '$lib/api/types';
import { groupItems } from '$lib/utilities/grouping';
import {
	SAMPLE, STILLS_CAP, frameDecided, frameFor, frameOf, guessShape, knownShape, measure, recordShape, sampleArt, shapeOf, showsOwnStill,
	splitByShape, stillKey, tallyStill, toldShape, usualStill, pictureShape
} from './shapes.svelte';

function item(id: string, title: string, kind = 'video', episode = 0, library = 'lib'): CatalogItem {
	return {
		id, library_id: library, kind, title, season: 0, episode, year: 0,
		file_path: `${id}.mkv`, size: 1, confidence: 1, origin: '', provenance: '', updated_at: 0
	};
}
const art = (id: string): Enrichment => ({ item_id: id, provider: 'p', remote_id: '', title: '', fetched_at: 0, poster: 'p.jpg' });
const banner = (id: string): Enrichment => ({ item_id: id, provider: 'p', remote_id: '', title: '', fetched_at: 0, cover: 'b.jpg' });

describe('shapes', () => {
	it('uses the guess until the picture is measured', () => {
		expect(shapeOf('a', 'landscape')).toBe('landscape');
		recordShape('a', 400, 600);
		expect(shapeOf('a', 'landscape')).toBe('portrait');
		recordShape('b', 1280, 720);
		expect(shapeOf('b', 'portrait')).toBe('landscape');
	});
	it('treats a near-square picture as portrait and ignores a broken one', () => {
		recordShape('c', 500, 480);
		expect(shapeOf('c', 'landscape')).toBe('portrait');
		recordShape('d', 0, 0);
		expect(shapeOf('d', 'landscape')).toBe('landscape');
	});
	it('guesses posters and reading files upright, a banner and a video still wide', () => {
		expect(guessShape(item('x', 'X'), null)).toBe('landscape');
		expect(guessShape(item('x', 'X'), art('x'))).toBe('portrait');
		expect(guessShape(item('x', 'X', 'manga'), null)).toBe('portrait');
		// A cover is the provider's banner: the card shows it when there is no poster.
		expect(guessShape(item('x', 'X'), banner('x'))).toBe('landscape');
		expect(guessShape(item('x', 'X', 'manga'), banner('x'))).toBe('landscape');
		expect(guessShape(item('x', 'X'), { ...banner('x'), poster: 'p.jpg' })).toBe('portrait');
	});
	it('guesses a video still by what its library\'s stills have been', () => {
		const film = (id: string) => item(id, id, 'video', 0, 'films');
		expect(guessShape(film('f0'), null)).toBe('landscape');
		// Two films came with posters, one without.
		recordShape('f1', 600, 900, film('f1'));
		recordShape('f2', 600, 900, film('f2'));
		recordShape('f3', 1280, 720, film('f3'));
		expect(guessShape(film('f4'), null)).toBe('portrait');
		// Counted once a visit: the same picture loaded again by another card.
		recordShape('f3', 1280, 720, film('f3'));
		recordShape('f3', 1280, 720, film('f3'));
		expect(guessShape(film('f4'), null)).toBe('portrait');
		// Overlay art and reading files are not the library's stills.
		expect(guessShape(film('f4'), banner('f4'))).toBe('landscape');
		expect(guessShape(item('m', 'M', 'manga', 0, 'films'), null)).toBe('portrait');
		// Another library learns nothing from this one.
		expect(guessShape(item('c', 'C', 'video', 0, 'clips'), null)).toBe('landscape');
		// A picture that is not the file's own still teaches nothing.
		recordShape('p1', 600, 900);
		expect(guessShape(item('c', 'C', 'video', 0, 'clips'), null)).toBe('landscape');
	});
	it('keeps a show\'s later episodes apart from its first', () => {
		const ep = (id: string, n: number) => item(id, 'Show', 'episode', n, 'shows');
		expect(stillKey(ep('e1', 1))).toBe('shows');
		expect(stillKey(item('f', 'F', 'video', 0, 'shows'))).toBe('shows');
		expect(stillKey(ep('e2', 2))).toBe('shows/later');
		// A later season's first episode is no show's card.
		expect(stillKey({ ...ep('s2e1', 1), season: 2 })).toBe('shows/later');
		expect(stillKey({ ...ep('s1e1', 1), season: 1 })).toBe('shows');
		// The first episode came with the show's poster; the others are frames.
		recordShape('s1e1', 600, 900, ep('s1e1', 1));
		for (let n = 2; n <= 6; n += 1) recordShape(`s1e${n}`, 1280, 720, ep(`s1e${n}`, n));
		expect(guessShape(ep('s2e1', 1), null)).toBe('portrait');
		expect(guessShape(ep('s2e3', 3), null)).toBe('landscape');
	});
	it('tallies stills, halving past the cap, a frame on a tie', () => {
		expect(usualStill(undefined)).toBeUndefined();
		expect(usualStill([0, 0])).toBeUndefined();
		expect(usualStill([1, 1])).toBe('landscape');
		expect(usualStill([2, 1])).toBe('portrait');
		expect(tallyStill(undefined, 'portrait')).toEqual([1, 0]);
		const full = tallyStill([STILLS_CAP, 0], 'landscape');
		expect(full[0] + full[1]).toBeLessThanOrEqual(STILLS_CAP);
		expect(usualStill(full)).toBe('portrait');
	});
	it('takes what the server tells of a file\'s own picture (D-282)', () => {
		const told = (id: string, source: string, aspect?: number, kind = 'video') => ({ ...item(id, id, kind, 0, 'told'), thumbnail: { source, aspect } });
		expect(toldShape(told('p', 'sidecar', 0.667))).toBe('portrait');
		expect(toldShape(told('w', 'sidecar', 1.778))).toBe('landscape');
		// Near square is upright, as a measured picture is.
		expect(toldShape(told('q', 'sidecar', 1.1))).toBe('portrait');
		expect(toldShape(told('c', 'cover', undefined, 'comic'))).toBe('portrait');
		// A frame is the video's shape, and a still whose header did not read says nothing.
		expect(toldShape(told('f', 'frame'))).toBeUndefined();
		expect(toldShape(told('s', 'sidecar'))).toBeUndefined();
		expect(toldShape(item('o', 'Old server'))).toBeUndefined();
		// Overlay art comes first, as the card shows it first.
		expect(knownShape(told('p', 'sidecar', 0.667), banner('p'))).toBe('landscape');
		expect(knownShape(told('f', 'frame'), null)).toBeUndefined();
		expect(guessShape(told('p2', 'sidecar', 0.667), null)).toBe('portrait');
		// Even in a library whose stills have been wide.
		for (let n = 0; n < 4; n += 1) recordShape(`tw${n}`, 1280, 720, item(`tw${n}`, 'W', 'video', 0, 'told'));
		expect(guessShape(told('p3', 'sidecar', 0.667), null)).toBe('portrait');
	});
	it('tallies a told frame apart from what was learned before the server told stills apart', () => {
		const frame = (id: string) => ({ ...item(id, id, 'video', 0, 'mixed'), thumbnail: { source: 'frame' } });
		expect(stillKey(frame('a'))).toBe('mixed/frames');
		// Learned from posters before D-282: the library's old tally is upright.
		for (let n = 0; n < 3; n += 1) recordShape(`mp${n}`, 600, 900, item(`mp${n}`, 'P', 'video', 0, 'mixed'));
		expect(guessShape(item('old', 'Old', 'video', 0, 'mixed'), null)).toBe('portrait');
		// A frame is not made upright by it.
		expect(guessShape(frame('b'), null)).toBe('landscape');
		// Upright phone clips are learned as frames.
		for (let n = 0; n < 3; n += 1) recordShape(`mf${n}`, 720, 1280, frame(`mf${n}`));
		expect(guessShape(frame('c'), null)).toBe('portrait');
	});
	it('tells a file\'s own still from overlay art and pages', () => {
		expect(showsOwnStill(item('x', 'X'), null)).toBe(true);
		expect(showsOwnStill(item('x', 'X'), art('x'))).toBe(false);
		expect(showsOwnStill(item('x', 'X'), banner('x'))).toBe(false);
		expect(showsOwnStill(item('x', 'X', 'comic'), null)).toBe(false);
	});
});

describe('frames', () => {
	it('takes the majority, portrait on a tie or with nothing to go by', () => {
		expect(frameFor(['landscape', 'landscape', 'portrait'])).toBe('landscape');
		expect(frameFor(['landscape', 'portrait'])).toBe('portrait');
		expect(frameFor([])).toBe('portrait');
	});
	it('is decided by the first titles only, so later pictures never reshape the grid', () => {
		const start: ('portrait' | 'landscape')[] = Array(SAMPLE).fill('portrait');
		expect(frameFor([...start, ...Array(SAMPLE * 3).fill('landscape')])).toBe('portrait');
	});
	it('reads each title by the picture its card shows, measured or guessed', () => {
		const groups = groupItems([
			item('s1', 'Show', 'episode', 1), item('s2', 'Show', 'episode', 2),
			item('f', 'Film'), item('g', 'Clip'), item('m', 'Manga', 'manga')
		]);
		const none = () => null;
		// Three stills (Show, Film, Clip) against one page.
		expect(frameOf(groups, none)).toBe('landscape');
		// The film has a poster and the show's first episode came with an upright still.
		recordShape('s1', 600, 900);
		expect(frameOf(groups, (id) => (id === 'f' ? art(id) : null))).toBe('portrait');
	});
	it('samples the art of the first titles only', () => {
		const groups = groupItems([item('s1', 'Show', 'episode', 1), item('s2', 'Show', 'episode', 2), item('f', 'Film')]);
		expect(sampleArt(groups).map((i) => i.id).sort()).toEqual(['f', 's1']);
		const many = groupItems(Array.from({ length: SAMPLE + 10 }, (_, i) => item(`i${i}`, `Title ${i}`)));
		expect(sampleArt(many)).toHaveLength(SAMPLE);
	});
	it('is decided once the measured pictures settle the majority', () => {
		const groups = groupItems(Array.from({ length: 5 }, (_, i) => item(`d${i}`, `D${i}`, 'video', 0, 'dec')));
		expect(frameDecided(groups)).toBe(false);
		recordShape('d0', 1280, 720);
		recordShape('d1', 1280, 720);
		expect(frameDecided(groups)).toBe(false);
		recordShape('d2', 1280, 720);
		// Three of five wide: the other two cannot turn it.
		expect(frameDecided(groups)).toBe(true);
		const upright = groupItems(Array.from({ length: 4 }, (_, i) => item(`u${i}`, `U${i}`, 'video', 0, 'dec')));
		recordShape('u0', 600, 900);
		expect(frameDecided(upright)).toBe(false);
		recordShape('u1', 600, 900);
		// Two of four upright: a tie at worst, and a tie is portrait.
		expect(frameDecided(upright)).toBe(true);
	});
	it('is decided by what is known before a picture is measured', () => {
		const told = (id: string, aspect: number) => ({ ...item(id, id, 'video', 0, 'known'), thumbnail: { source: 'sidecar', aspect } });
		const groups = groupItems([told('k0', 0.667), told('k1', 0.667), item('k2', 'K2', 'video', 0, 'known')]);
		expect(frameDecided(groups)).toBe(true);
		expect(frameOf(groups, () => null)).toBe('portrait');
		const unknown = groupItems([told('k3', 0.667), item('k4', 'K4', 'video', 0, 'known'), item('k5', 'K5', 'video', 0, 'known')]);
		expect(frameDecided(unknown)).toBe(false);
		// Overlay art counts as known.
		expect(frameDecided(unknown, (id) => (id === 'k4' ? art(id) : null))).toBe(true);
	});
	it('measures nothing outside a browser and nothing already known', async () => {
		await expect(measure([{ id: 's1', src: 'x.jpg' }, { id: 'new', src: '' }])).resolves.toBeUndefined();
	});
});

describe('splitByShape', () => {
	const shape = (s: string) => (s.startsWith('w') ? 'landscape' : 'portrait');
	it('keeps each part in order, the larger part first', () => {
		expect(splitByShape(['p1', 'w1', 'p2', 'w2', 'w3'], shape)).toEqual([
			{ shape: 'landscape', items: ['w1', 'w2', 'w3'] },
			{ shape: 'portrait', items: ['p1', 'p2'] }
		]);
	});
	it('puts posters first on a tie and leaves an empty part out', () => {
		expect(splitByShape(['w1', 'p1'], shape).map((p) => p.shape)).toEqual(['portrait', 'landscape']);
		expect(splitByShape(['p1', 'p2'], shape)).toEqual([{ shape: 'portrait', items: ['p1', 'p2'] }]);
		expect(splitByShape([], shape)).toEqual([]);
	});
});

describe('a title in a poster frame', () => {
	// The owner's Animes rail: the overlay is on the episode under way, the
	// first episode has none and its picture is a wide frame. The card is the
	// title's poster, whatever episode is being watched.
	// The first episode's own picture is wide (its frame, as the server tells it).
	const first = { ...item('pf1', 'Show', 'episode', 1, 'anime'), thumbnail: { source: 'sidecar', aspect: 1.778 } };
	const show = groupItems([first, item('pf6', 'Show', 'episode', 6, 'anime'), item('pf7', 'Show', 'episode', 7, 'anime')])[0];
	const overlays = new Map([['pf6', art('pf6')]]);
	it('draws the poster any file of the title has', () => {
		expect(pictureShape(show, (id) => overlays.get(id) ?? null)).toBe('portrait');
	});
});
