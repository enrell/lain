import { describe, expect, it } from 'vitest';
import type { CatalogItem, Enrichment } from '$lib/api/types';
import { groupItems } from '$lib/utilities/grouping';
import { SAMPLE, frameFor, frameOf, guessShape, recordShape, shapeOf } from './shapes.svelte';

function item(id: string, title: string, kind = 'video', episode = 0): CatalogItem {
	return {
		id, library_id: 'lib', kind, title, season: 0, episode, year: 0,
		file_path: `${id}.mkv`, size: 1, confidence: 1, origin: '', provenance: '', updated_at: 0
	};
}
const art = (id: string): Enrichment => ({ item_id: id, provider: 'p', remote_id: '', title: '', fetched_at: 0, poster: 'p.jpg' });

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
	it('guesses art and reading files upright and a video still wide', () => {
		expect(guessShape(item('x', 'X'), null)).toBe('landscape');
		expect(guessShape(item('x', 'X'), art('x'))).toBe('portrait');
		expect(guessShape(item('x', 'X', 'manga'), null)).toBe('portrait');
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
		// The film has a poster and the show's first episode loads upright.
		recordShape('s1', 600, 900);
		expect(frameOf(groups, (id) => (id === 'f' ? art(id) : null))).toBe('portrait');
	});
});
