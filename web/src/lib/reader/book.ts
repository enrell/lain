/*
 * Pure logic of the book reader (D-306, D-307): where a reading position
 * sits in the book's virtual pages, how a chapter's resources get the
 * session token, and the stylesheet the reader lays over a chapter.
 * Kept apart from the component so it is testable without a browser.
 */

import type { BookChapter } from '$lib/api/types';

/** Characters in one virtual page (`BOOK_PAGE_CHARS` on the server). */
export const PAGE_CHARS = 1024;

/** A place in a reflowable book: a chapter and how far into it (0..1). */
export interface BookPosition {
	chapter: number;
	fraction: number;
}

export interface BookProgress {
	position_sec: number;
	duration_sec: number;
	completed: boolean;
}

/** Characters before each chapter, and the total at the end. */
export function offsets(chapters: Pick<BookChapter, 'chars'>[]): number[] {
	const out = [0];
	for (const c of chapters) out.push(out[out.length - 1] + Math.max(0, c.chars));
	return out;
}

/** Virtual pages of the whole book (at least one). */
export function totalPages(chapters: Pick<BookChapter, 'chars'>[]): number {
	const total = offsets(chapters).at(-1) ?? 0;
	return Math.max(1, Math.ceil(total / PAGE_CHARS));
}

/**
 * The progress record for a position: `position_sec` is the 1-based
 * virtual page, fractional so the exact place survives (D-307),
 * `duration_sec` the page count. The last page of the last chapter
 * finishes the book.
 */
export function progressAt(pos: BookPosition, chapters: Pick<BookChapter, 'chars'>[]): BookProgress {
	const off = offsets(chapters);
	const n = chapters.length;
	const ch = Math.min(Math.max(pos.chapter, 0), Math.max(n - 1, 0));
	const f = Math.min(Math.max(pos.fraction, 0), 1);
	const chars = (off[ch] ?? 0) + f * Math.max(0, chapters[ch]?.chars ?? 0);
	const pages = totalPages(chapters);
	const position = Math.min(1 + chars / PAGE_CHARS, pages);
	const completed = n > 0 && ch === n - 1 && f >= 0.98;
	return { position_sec: Math.round(position * 1000) / 1000, duration_sec: pages, completed };
}

/**
 * Where a progress record points: the chapter holding that character,
 * and how far into it. A finished or unread book opens at the start.
 */
export function positionOf(
	progress: { position_sec: number; duration_sec: number; completed: boolean } | null,
	chapters: Pick<BookChapter, 'chars'>[]
): BookPosition {
	if (!progress || progress.completed || progress.position_sec <= 1 || chapters.length === 0) {
		return { chapter: 0, fraction: 0 };
	}
	const off = offsets(chapters);
	const chars = (progress.position_sec - 1) * PAGE_CHARS;
	for (let i = 0; i < chapters.length; i++) {
		const len = Math.max(0, chapters[i].chars);
		// A saved place right at a chapter's start, rounded on the way,
		// must not reopen at the end of the chapter before it.
		if (chars < off[i] + len - 0.75 || i === chapters.length - 1) {
			if (len === 0) continue;
			return { chapter: i, fraction: Math.min(Math.max((chars - off[i]) / len, 0), 1) };
		}
	}
	return { chapter: chapters.length - 1, fraction: 1 };
}

/** The chapter holding virtual page `page` (1-based), at its place. */
export function positionOfPage(page: number, chapters: Pick<BookChapter, 'chars'>[]): BookPosition {
	return positionOf({ position_sec: Math.max(page, 1), duration_sec: 0, completed: false }, chapters);
}

/**
 * Appends the session token to every resource URL of a chapter (the
 * server writes them as `/api/items/{id}/book/res/<path>` with the path
 * percent-encoded, so they hold no quote, space or parenthesis).
 */
export function withToken(html: string, itemId: string, token: string | null): string {
	if (!token) return html;
	const prefix = `/api/items/${itemId}/book/res/`.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
	// Only a URL that starts the attribute value or the CSS url() (a
	// root-relative one, as the server writes them): never one inside a
	// link to another site that happens to contain the same path.
	const re = new RegExp(`(["'(]\\s*)(${prefix}[^"'()\\s<>?]*)`, 'g');
	return html.replace(re, `$1$2?token=${encodeURIComponent(token)}`);
}

export type BookFont = 'book' | 'serif' | 'sans' | 'mono';
export type BookTheme = 'auto' | 'paper' | 'light' | 'dark' | 'original';
export type BookLayout = 'paged' | 'scroll';

export interface BookLook {
	layout: BookLayout;
	/** Percent of the book's own size. */
	size: number;
	font: BookFont;
	lineHeight: number;
	/** Side margin in CSS pixels. */
	margin: number;
	justify: boolean;
	theme: BookTheme;
	/** Colours of the `auto` theme: the app's own. */
	appColors?: { background: string; foreground: string; accent: string };
}

const FONTS: Record<Exclude<BookFont, 'book'>, string> = {
	serif: "'Iowan Old Style', 'Palatino Linotype', Palatino, Georgia, 'Noto Serif', serif",
	sans: "system-ui, -apple-system, 'Segoe UI', Roboto, 'Noto Sans', sans-serif",
	mono: "ui-monospace, 'JetBrains Mono', 'SF Mono', Menlo, Consolas, monospace"
};

const THEMES: Record<Exclude<BookTheme, 'auto' | 'original'>, { background: string; foreground: string; accent: string }> = {
	paper: { background: '#f4ecd8', foreground: '#2b2118', accent: '#8a4b14' },
	light: { background: '#ffffff', foreground: '#1b1b1b', accent: '#1f5fbf' },
	dark: { background: '#141414', foreground: '#d8d4cc', accent: '#8ab4f8' }
};

/** The colours a look paints with; `null` keeps the book's own. */
export function colorsOf(look: BookLook): { background: string; foreground: string; accent: string } | null {
	if (look.theme === 'original') return null;
	if (look.theme === 'auto') return look.appColors ?? THEMES.dark;
	return THEMES[look.theme];
}

/**
 * The stylesheet laid over a chapter. In the paged layout the body
 * flows into columns one viewport wide, so turning a page is scrolling
 * the document by one viewport; `width` and `height` are the frame's.
 */
export function readerCss(look: BookLook, width: number, height: number): string {
	const colors = colorsOf(look);
	// A line longer than about 75 characters is hard to follow: on a wide
	// screen the margins grow so the page keeps a readable measure.
	const measure = Math.round(38 * 16 * (look.size / 100) * 1.25);
	const m = Math.max(0, Math.round(look.layout === 'paged' ? Math.max(look.margin, (width - measure) / 2) : look.margin));
	const vertical = Math.max(16, Math.round(Math.min(m, 48)));
	const rules: string[] = [];
	rules.push(`html { font-size: ${Math.round(look.size)}% !important; -webkit-text-size-adjust: none; text-size-adjust: none; }`);
	const body = [
		'margin: 0 !important',
		`line-height: ${look.lineHeight} !important`,
		'overflow-wrap: break-word',
		'hyphens: auto',
		'-webkit-hyphens: auto'
	];
	if (look.font !== 'book') body.push(`font-family: ${FONTS[look.font]} !important`);
	if (look.justify) body.push('text-align: justify');
	if (colors) {
		body.push(`background: ${colors.background} !important`, `color: ${colors.foreground} !important`);
		rules.push(`html { background: ${colors.background} !important; color-scheme: ${isDark(colors.background) ? 'dark' : 'light'}; }`);
		// The book's own colours would fight the theme: text and boxes inherit it.
		rules.push('body *:not(img):not(svg):not(svg *) { color: inherit !important; background-color: transparent !important; border-color: currentColor !important; }');
		rules.push(`a, a * { color: ${colors.accent} !important; }`);
	}
	if (look.font !== 'book') rules.push(`body p, body li, body div, body span, body blockquote, body td { font-family: inherit !important; }`);
	if (look.layout === 'paged') {
		const columnWidth = Math.max(100, width - 2 * m);
		rules.push(`html { height: ${height}px !important; overflow: hidden !important; }`);
		body.push(
			`height: ${height}px !important`,
			'box-sizing: border-box !important',
			`padding: ${vertical}px ${m}px !important`,
			`column-width: ${columnWidth}px !important`,
			`column-gap: ${2 * m}px !important`,
			'column-fill: auto !important',
			'overflow: visible !important'
		);
		rules.push(`img, svg, video { max-width: 100% !important; max-height: ${Math.max(100, height - 2 * vertical)}px !important; height: auto; object-fit: contain; break-inside: avoid; }`);
		rules.push('h1, h2, h3, h4 { break-after: avoid; }');
	} else {
		body.push(`padding: ${vertical}px ${m}px ${vertical * 3}px !important`, 'max-width: 42em !important', 'margin: 0 auto !important', 'box-sizing: content-box');
		rules.push('img, svg, video { max-width: 100% !important; height: auto; }');
	}
	rules.push(`body { ${body.join('; ')}; }`);
	rules.push('::selection { background: rgba(127, 127, 127, 0.35); }');
	return rules.join('\n');
}

function isDark(hex: string): boolean {
	const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(hex.trim());
	if (!m) return true;
	const [r, g, b] = [m[1], m[2], m[3]].map((x) => parseInt(x, 16));
	return 0.299 * r + 0.587 * g + 0.114 * b < 128;
}

/** Pages of a paged chapter: its scroll width over one viewport. */
export function pagesIn(scrollWidth: number, viewport: number): number {
	if (viewport <= 0) return 1;
	return Math.max(1, Math.round(scrollWidth / viewport));
}
