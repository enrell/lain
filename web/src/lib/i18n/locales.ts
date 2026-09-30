/**
 * The locale registry. Adding a language is: add an entry here, add
 * `messages/<code>.ts` with the same keys as `en.ts` (the parity test
 * enforces it), done. See docs/I18N.md.
 */
export interface LocaleInfo {
	/** BCP 47 tag, also the catalog file name. */
	code: string;
	/** The language's own name for itself ("Português", not "Portuguese"). */
	name: string;
	dir: 'ltr' | 'rtl';
	/** Development-only locales are hidden from users. */
	dev?: boolean;
}

/** The language the source catalog is written in; every key falls back to it. */
export const SOURCE_LOCALE = 'en';

export const LOCALES: LocaleInfo[] = [
	{ code: 'en', name: 'English', dir: 'ltr' },
	// Pseudo-locales: generated from English at runtime to find strings that
	// bypass t(), truncation and mirroring bugs before a real translation.
	{ code: 'en-XA', name: 'Pseudo (accented, long)', dir: 'ltr', dev: true },
	{ code: 'ar-XB', name: 'Pseudo (right to left)', dir: 'rtl', dev: true }
];

export const PSEUDO_LOCALES = new Set(['en-XA', 'ar-XB']);

export function localeInfo(code: string): LocaleInfo {
	return LOCALES.find((l) => l.code === code) ?? LOCALES[0];
}
