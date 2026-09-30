import { describe, expect, it } from 'vitest';
import {
	buildSpreads,
	keyAction,
	progressFor,
	resolveDirection,
	spreadContaining,
	startPage,
	tapAction,
	visualOrder,
	type ReaderPage
} from './layout';
import { isReadable, readingLabel } from './kinds';

const page = (index: number, width = 800, height = 1200): ReaderPage => ({
	index,
	mime: 'image/jpeg',
	size: 1,
	width,
	height
});
const pages = (n: number) => Array.from({ length: n }, (_, i) => page(i));

describe('buildSpreads', () => {
	it('shows single pages when dual is off', () => {
		expect(buildSpreads(pages(3), { dual: false, coverAlone: true })).toEqual([[0], [1], [2]]);
	});
	it('keeps the cover alone and pairs the rest', () => {
		expect(buildSpreads(pages(6), { dual: true, coverAlone: true })).toEqual([
			[0],
			[1, 2],
			[3, 4],
			[5]
		]);
	});
	it('pairs from the first page when the cover is not alone', () => {
		expect(buildSpreads(pages(4), { dual: true, coverAlone: false })).toEqual([[0, 1], [2, 3]]);
	});
	it('never pairs a wide page, on either side', () => {
		const p = [page(0), page(1), page(2, 1600, 1000), page(3), page(4)];
		expect(buildSpreads(p, { dual: true, coverAlone: true })).toEqual([[0], [1], [2], [3, 4]]);
	});
	it('treats unknown dimensions as portrait', () => {
		const p = [page(0, 0, 0), page(1, 0, 0), page(2, 0, 0)];
		expect(buildSpreads(p, { dual: true, coverAlone: true })).toEqual([[0], [1, 2]]);
	});
	it('covers every page exactly once', () => {
		const p = pages(11);
		const flat = buildSpreads(p, { dual: true, coverAlone: true }).flat();
		expect(flat).toEqual(p.map((x) => x.index));
	});
	it('handles empty and single-page archives', () => {
		expect(buildSpreads([], { dual: true, coverAlone: true })).toEqual([]);
		expect(buildSpreads(pages(1), { dual: true, coverAlone: false })).toEqual([[0]]);
	});
});

describe('direction', () => {
	it('lays a manga spread out right to left', () => {
		expect(visualOrder([4, 5], 'rtl')).toEqual([5, 4]);
		expect(visualOrder([4, 5], 'ltr')).toEqual([4, 5]);
	});
	it('maps taps to the reading direction', () => {
		expect(tapAction(10, 300, 'rtl')).toBe('next');
		expect(tapAction(290, 300, 'rtl')).toBe('prev');
		expect(tapAction(10, 300, 'ltr')).toBe('prev');
		expect(tapAction(290, 300, 'ltr')).toBe('next');
		expect(tapAction(150, 300, 'rtl')).toBe('toggle-ui');
		expect(tapAction(1, 0, 'ltr')).toBeNull();
	});
	it('maps arrows to the reading direction', () => {
		expect(keyAction('ArrowLeft', false, 'rtl')).toBe('next');
		expect(keyAction('ArrowLeft', false, 'ltr')).toBe('prev');
		expect(keyAction('ArrowRight', false, 'rtl')).toBe('prev');
		expect(keyAction(' ', true, 'ltr')).toBe('prev');
		expect(keyAction('End', false, 'ltr')).toBe('last');
		expect(keyAction('q', false, 'ltr')).toBeNull();
	});
	it('prefers a saved direction over the default', () => {
		expect(resolveDirection('ltr', 'rtl')).toBe('ltr');
		expect(resolveDirection('bogus', 'rtl')).toBe('rtl');
		expect(resolveDirection(null, 'ltr')).toBe('ltr');
	});
});

describe('progress', () => {
	it('records the 1-based page and completion on the last page', () => {
		expect(progressFor(0, 10)).toEqual({ position_sec: 1, duration_sec: 10, completed: false });
		expect(progressFor(9, 10)).toEqual({ position_sec: 10, duration_sec: 10, completed: true });
		expect(progressFor(99, 10).position_sec).toBe(10);
	});
	it('resumes at the saved page and restarts finished ones', () => {
		expect(startPage({ position_sec: 5, duration_sec: 10, completed: false }, 10)).toBe(4);
		expect(startPage({ position_sec: 10, duration_sec: 10, completed: true }, 10)).toBe(0);
		expect(startPage(null, 10)).toBe(0);
		expect(startPage({ position_sec: 50, duration_sec: 10, completed: false }, 10)).toBe(9);
	});
	it('finds the spread holding a page', () => {
		const s = buildSpreads(pages(6), { dual: true, coverAlone: true });
		expect(spreadContaining(s, 4)).toBe(2);
		expect(spreadContaining(s, 99)).toBe(0);
	});
});

describe('kinds', () => {
	it('knows which kinds open in the reader', () => {
		expect(isReadable('manga')).toBe(true);
		expect(isReadable('comic')).toBe(true);
		expect(isReadable('anime')).toBe(false);
		expect(isReadable(undefined)).toBe(false);
	});
	it('labels volumes, chapters and issues', () => {
		const base = { title: 'T' };
		expect(readingLabel({ ...base, kind: 'manga', season: 3, episode: 0 })).toBe('Vol 3');
		expect(readingLabel({ ...base, kind: 'manga', season: 0, episode: 12 })).toBe('Ch 12');
		expect(readingLabel({ ...base, kind: 'manga', season: 2, episode: 15 })).toBe('Vol 2 · Ch 15');
		expect(readingLabel({ ...base, kind: 'comic', season: 0, episode: 4 })).toBe('Issue 4');
		expect(readingLabel({ ...base, kind: 'comic', season: 0, episode: 0 })).toBe('T');
	});
});
