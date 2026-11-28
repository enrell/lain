import { describe, expect, it } from 'vitest';
import {
	PAGE_CHARS,
	colorsOf,
	offsets,
	pagesIn,
	positionOf,
	positionOfPage,
	progressAt,
	readerCss,
	totalPages,
	withToken,
	type BookLook
} from './book';

const chapters = [{ chars: 0 }, { chars: 2048 }, { chars: 1024 }, { chars: 512 }];

describe('virtual pages', () => {
	it('counts characters before each chapter', () => {
		expect(offsets(chapters)).toEqual([0, 0, 2048, 3072, 3584]);
		expect(totalPages(chapters)).toBe(4);
		expect(totalPages([])).toBe(1);
	});

	it('maps a position to a fractional page and back', () => {
		const p = progressAt({ chapter: 1, fraction: 0.5 }, chapters);
		expect(p).toEqual({ position_sec: 2, duration_sec: 4, completed: false });
		expect(positionOf(p, chapters)).toEqual({ chapter: 1, fraction: 0.5 });
		const q = progressAt({ chapter: 2, fraction: 0.25 }, chapters);
		expect(q.position_sec).toBe(1 + (2048 + 256) / PAGE_CHARS);
		expect(positionOf(q, chapters)).toEqual({ chapter: 2, fraction: 0.25 });
	});

	it('finishes at the end of the last chapter and reopens a finished book at the start', () => {
		const end = progressAt({ chapter: 3, fraction: 1 }, chapters);
		expect(end.completed).toBe(true);
		expect(end.position_sec).toBeLessThanOrEqual(end.duration_sec);
		expect(positionOf(end, chapters)).toEqual({ chapter: 0, fraction: 0 });
		expect(positionOf(null, chapters)).toEqual({ chapter: 0, fraction: 0 });
	});

	it('skips empty chapters when resuming and clamps past the end', () => {
		// Page 1 exactly is the start; a cover chapter with no text is not a landing place.
		expect(positionOf({ position_sec: 1.5, duration_sec: 4, completed: false }, chapters)).toEqual({ chapter: 1, fraction: 0.25 });
		expect(positionOfPage(99, chapters)).toEqual({ chapter: 3, fraction: 1 });
		expect(positionOfPage(3, chapters)).toEqual({ chapter: 2, fraction: 0 });
	});
});

describe('resource URLs', () => {
	it('gets the token on every resource of this item only', () => {
		const html =
			'<img src="/api/items/abc/book/res/OEBPS/a%20b.png"><style>p{background:url("/api/items/abc/book/res/OEBPS/x.png")}</style><a href="/api/items/zzz/book/res/y.png">';
		const out = withToken(html, 'abc', 't k');
		expect(out).toContain('/api/items/abc/book/res/OEBPS/a%20b.png?token=t%20k"');
		expect(out).toContain('url("/api/items/abc/book/res/OEBPS/x.png?token=t%20k")');
		expect(out).toContain('/api/items/zzz/book/res/y.png"');
		expect(withToken(html, 'abc', null)).toBe(html);
		// A link elsewhere that contains the same path never gets the token.
		const evil = '<a href="https://evil.example/api/items/abc/book/res/x.png">';
		expect(withToken(evil, 'abc', 'secret')).toBe(evil);
	});
});

describe('the reader stylesheet', () => {
	const look: BookLook = { layout: 'paged', size: 120, font: 'serif', lineHeight: 1.6, margin: 40, justify: true, theme: 'paper' };

	it('lays the body into viewport-wide columns', () => {
		const css = readerCss(look, 800, 600);
		expect(css).toContain('column-width: 720px');
		expect(css).toContain('column-gap: 80px');
		// A wide screen keeps a readable line.
		const wide = readerCss(look, 2000, 600);
		expect(wide).toContain('column-width: 912px');
		expect(css).toContain('font-size: 120%');
		expect(css).toContain('#f4ecd8');
		expect(css).toContain('text-align: justify');
	});

	it('keeps the book colours with the original theme and scrolls in the scroll layout', () => {
		const css = readerCss({ ...look, theme: 'original', layout: 'scroll', font: 'book' }, 800, 600);
		expect(css).not.toContain('column-width');
		expect(css).not.toContain('background-color: transparent');
		expect(css).not.toContain('font-family');
		expect(colorsOf({ ...look, theme: 'auto', appColors: { background: '#000', foreground: '#fff', accent: '#f00' } })?.accent).toBe('#f00');
	});

	it('counts the pages of a paged chapter', () => {
		expect(pagesIn(2400, 800)).toBe(3);
		expect(pagesIn(2390, 800)).toBe(3);
		expect(pagesIn(0, 800)).toBe(1);
		expect(pagesIn(100, 0)).toBe(1);
	});
});
