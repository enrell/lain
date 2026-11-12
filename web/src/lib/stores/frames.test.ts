import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CatalogItem } from '$lib/api/types';
import { groupItems } from '$lib/utilities/grouping';
import { guessFrame, settleFrame } from './frames';
import { enrichmentCache, libraryCache } from './media-cache.svelte';
import { guessShape, MEASURE_WAIT_MS, measure } from './shapes.svelte';

/*
 * A grid is drawn in `guessFrame` before any picture loads and corrected
 * once by `settleFrame`. These hold that order: a guess that waited for
 * pictures, or a check that waited for every picture after the first ones
 * had decided it, left a cold grid blank or a wrong frame standing for
 * seconds, and both read the same in the store's pure helpers.
 */

/** An `Image` that answers only when the test says so. */
class FakeImage {
	static made: FakeImage[] = [];
	onload: (() => void) | null = null;
	onerror: (() => void) | null = null;
	naturalWidth = 0;
	naturalHeight = 0;
	decoding = '';
	referrerPolicy = '';
	src = '';
	constructor() {
		FakeImage.made.push(this);
	}
	answer(width: number, height: number): void {
		this.naturalWidth = width;
		this.naturalHeight = height;
		this.onload?.();
	}
}

function video(id: string, library: string): CatalogItem {
	return {
		id, library_id: library, kind: 'video', title: id, season: 0, episode: 0, year: 0,
		file_path: `${id}.mkv`, size: 1, confidence: 1, origin: '', provenance: '', updated_at: 0
	};
}
/** Films of one library with no overlay art: their cards show the files' own stills. */
function films(prefix: string, library: string, n: number) {
	const items = Array.from({ length: n }, (_, i) => video(`${prefix}${i}`, library));
	// Known to have no overlay, so nothing is fetched.
	for (const i of items) enrichmentCache.set(i.id, null);
	return groupItems(items);
}
async function flush(): Promise<void> {
	for (let i = 0; i < 10; i += 1) await Promise.resolve();
}

beforeEach(() => {
	FakeImage.made = [];
	vi.stubGlobal('Image', FakeImage);
	vi.useFakeTimers();
});
afterEach(() => {
	vi.useRealTimers();
	vi.unstubAllGlobals();
});

describe('a set drawn before its pictures', () => {
	it('is guessed without loading a picture', async () => {
		const groups = films('g', 'guess', 3);
		await expect(guessFrame(groups)).resolves.toBe('landscape');
		expect(FakeImage.made).toHaveLength(0);
	});

	it('is corrected as soon as its first pictures decide it, not when the last answers', async () => {
		const groups = films('c', 'check', 3);
		const settled = settleFrame(groups);
		await flush();
		expect(FakeImage.made).toHaveLength(3);
		// Two of three came with posters: the third, a frame still cut by
		// ffmpeg, cannot turn it. Fake timers: only an early answer resolves.
		FakeImage.made[0].answer(600, 900);
		FakeImage.made[1].answer(600, 900);
		await expect(settled).resolves.toBe('portrait');
	});

	it('keeps the guess when the pictures do not answer in time', async () => {
		const groups = films('t', 'slow', 3);
		const settled = settleFrame(groups);
		await flush();
		vi.advanceTimersByTime(MEASURE_WAIT_MS);
		await expect(settled).resolves.toBe('landscape');
	});

	it('learns a public library\'s stills for the next guess, never a private one\'s', async () => {
		libraryCache.set('pub', { id: 'pub', name: 'Pub', type: 'auto', path: '', source: '', created_at: 0 });
		libraryCache.set('priv', { id: 'priv', name: 'Priv', type: 'auto', path: '', source: '', created_at: 0, private: true });
		const pub = settleFrame(films('p', 'pub', 2));
		const priv = settleFrame(films('q', 'priv', 2));
		await flush();
		for (const img of FakeImage.made) img.answer(600, 900);
		await Promise.all([pub, priv]);
		expect(guessShape(video('p-next', 'pub'), null)).toBe('portrait');
		expect(guessShape(video('q-next', 'priv'), null)).toBe('landscape');
	});

	it('measures each picture once', async () => {
		const done = measure([{ id: 'm0', src: 'm0.jpg' }, { id: 'm1', src: '' }], MEASURE_WAIT_MS);
		expect(FakeImage.made.map((i) => i.src)).toEqual(['m0.jpg']);
		FakeImage.made[0].answer(1280, 720);
		await done;
		await measure([{ id: 'm0', src: 'm0.jpg' }]);
		expect(FakeImage.made).toHaveLength(1);
	});
});
