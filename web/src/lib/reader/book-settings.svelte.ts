import { prefs } from '$lib/auth/storage';
import type { BookFont, BookLayout, BookLook, BookTheme } from './book';

export const SIZE_MIN = 70;
export const SIZE_MAX = 220;
const SIZE_STEP = 10;
const LINE_HEIGHTS = [1.3, 1.5, 1.7, 2];
const MARGINS = [16, 40, 80];
const FONTS: BookFont[] = ['book', 'serif', 'sans', 'mono'];
const THEMES: BookTheme[] = ['auto', 'paper', 'light', 'dark', 'original'];

/**
 * The book reader's look, personal and applied at once (D-087): one set
 * for every book, kept in this browser like the comic reader's view.
 */
export class BookSettings {
	layout = $state<BookLayout>('paged');
	size = $state(100);
	font = $state<BookFont>('book');
	lineHeight = $state(1.5);
	margin = $state(40);
	justify = $state(true);
	theme = $state<BookTheme>('auto');

	constructor() {
		const layout = prefs.get('reader.book.layout');
		this.layout = layout === 'scroll' || layout === 'paged' ? layout : 'paged';
		const size = Number(prefs.get('reader.book.size'));
		this.size = Number.isFinite(size) && size >= SIZE_MIN && size <= SIZE_MAX ? size : 100;
		const font = prefs.get('reader.book.font') as BookFont | null;
		this.font = font && FONTS.includes(font) ? font : 'book';
		const lh = Number(prefs.get('reader.book.line_height'));
		this.lineHeight = LINE_HEIGHTS.includes(lh) ? lh : 1.5;
		const margin = Number(prefs.get('reader.book.margin'));
		this.margin = MARGINS.includes(margin) ? margin : 40;
		this.justify = prefs.getBool('reader.book.justify', true);
		const theme = prefs.get('reader.book.theme') as BookTheme | null;
		this.theme = theme && THEMES.includes(theme) ? theme : 'auto';
	}

	look(appColors?: BookLook['appColors']): BookLook {
		return {
			layout: this.layout,
			size: this.size,
			font: this.font,
			lineHeight: this.lineHeight,
			margin: this.margin,
			justify: this.justify,
			theme: this.theme,
			appColors
		};
	}

	setLayout(l: BookLayout): void {
		this.layout = l;
		prefs.set('reader.book.layout', l);
	}
	toggleLayout(): void {
		this.setLayout(this.layout === 'paged' ? 'scroll' : 'paged');
	}
	setSize(s: number): void {
		this.size = Math.min(SIZE_MAX, Math.max(SIZE_MIN, Math.round(s / SIZE_STEP) * SIZE_STEP));
		prefs.set('reader.book.size', String(this.size));
	}
	bigger(): void {
		this.setSize(this.size + SIZE_STEP);
	}
	smaller(): void {
		this.setSize(this.size - SIZE_STEP);
	}
	setFont(f: BookFont): void {
		this.font = f;
		prefs.set('reader.book.font', f);
	}
	setLineHeight(v: number): void {
		this.lineHeight = v;
		prefs.set('reader.book.line_height', String(v));
	}
	setMargin(v: number): void {
		this.margin = v;
		prefs.set('reader.book.margin', String(v));
	}
	setJustify(v: boolean): void {
		this.justify = v;
		prefs.set('reader.book.justify', String(v));
	}
	setTheme(t: BookTheme): void {
		this.theme = t;
		prefs.set('reader.book.theme', t);
	}

	static readonly fonts = FONTS;
	static readonly themes = THEMES;
	static readonly lineHeights = LINE_HEIGHTS;
	static readonly margins = MARGINS;
}
