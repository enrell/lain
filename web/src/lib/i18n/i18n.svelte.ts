/*
 * Runtime i18n: the active locale and its catalog as Svelte state, so
 * every `t()` in a template re-renders when the language changes.
 *
 *   import { t } from '$lib/i18n';
 *   t('nav.home')                              → "Home"
 *   t('settings.staged.count', { count: 3 })   → "3 changes staged"
 *
 * Formatting (dates, numbers, relative time, lists) goes through the
 * same locale via `i18n.format*`, never through toLocale*(undefined).
 */
import { prefs } from '$lib/auth/storage';
import {
	interpolate,
	isPlural,
	lookup,
	pseudoTree,
	resolveLocale,
	selectPlural,
	type MessageKey,
	type MessageTree,
	type Params
} from './core';
import { LOCALES, PSEUDO_LOCALES, SOURCE_LOCALE, localeInfo } from './locales';
import en from './messages/en';

export type Messages = typeof en;
export type Key = MessageKey<Messages>;

const PREF = 'locale';

// Catalogs other than the source load on demand: one chunk per language.
const loaders = import.meta.glob<{ default: MessageTree }>(['./messages/*.ts', '!./messages/en.ts']);

async function loadCatalog(code: string): Promise<MessageTree> {
	if (code === SOURCE_LOCALE) return en as unknown as MessageTree;
	if (PSEUDO_LOCALES.has(code)) return pseudoTree(en as unknown as MessageTree);
	const load = loaders[`./messages/${code}.ts`];
	if (!load) return en as unknown as MessageTree;
	return (await load()).default;
}

class I18n {
	locale = $state(SOURCE_LOCALE);
	/** '' means "follow the browser". */
	choice = $state('');
	#catalog = $state<MessageTree>(en as unknown as MessageTree);
	#missing = new Set<string>();

	get dir(): 'ltr' | 'rtl' {
		return localeInfo(this.locale).dir;
	}

	/** Locales a user may pick; development locales only in dev builds or when already active. */
	get available() {
		return LOCALES.filter((l) => !l.dev || import.meta.env.DEV || l.code === this.locale);
	}

	/** Resolves and loads the locale. Call once before the first render. */
	async init(): Promise<void> {
		const fromUrl = typeof location !== 'undefined' ? new URLSearchParams(location.search).get('locale') : null;
		this.choice = fromUrl ?? prefs.get(PREF) ?? '';
		await this.#apply(this.#resolve());
	}

	/** Pick a language ('' = follow the browser) and remember it here. */
	async setLocale(choice: string): Promise<void> {
		this.choice = choice;
		prefs.set(PREF, choice);
		await this.#apply(this.#resolve());
	}

	#resolve(): string {
		const browser = typeof navigator !== 'undefined' ? navigator.languages ?? [navigator.language] : [];
		return resolveLocale(this.choice, browser, LOCALES.map((l) => l.code), SOURCE_LOCALE);
	}

	/** The locale the browser alone would pick, for the "System (…)" label. */
	get systemLocale(): string {
		const browser = typeof navigator !== 'undefined' ? navigator.languages ?? [navigator.language] : [];
		return resolveLocale(null, browser, LOCALES.filter((l) => !l.dev).map((l) => l.code), SOURCE_LOCALE);
	}

	async #apply(code: string): Promise<void> {
		this.#catalog = await loadCatalog(code);
		this.locale = code;
		this.#rules = new Intl.PluralRules(this.#intlLocale);
		this.#number = new Intl.NumberFormat(this.#intlLocale);
		if (typeof document !== 'undefined') {
			document.documentElement.lang = code;
			document.documentElement.dir = this.dir;
		}
	}

	/** Pseudo locales format like their base language. */
	get #intlLocale(): string {
		return this.locale === 'ar-XB' ? 'en' : this.locale.replace('-XA', '');
	}
	#rules = new Intl.PluralRules(SOURCE_LOCALE);
	#number = new Intl.NumberFormat(SOURCE_LOCALE);

	t = (key: Key, params?: Params): string => {
		const value = lookup(this.#catalog, key) ?? lookup(en as unknown as MessageTree, key);
		if (value === undefined) {
			if (import.meta.env.DEV && !this.#missing.has(key)) {
				this.#missing.add(key);
				console.warn(`[i18n] missing key "${key}"`);
			}
			return key;
		}
		const fmt = (n: number) => this.#number.format(n);
		if (isPlural(value)) {
			const count = typeof params?.count === 'number' ? params.count : 0;
			return interpolate(selectPlural(value, count, this.#rules), params, fmt);
		}
		return interpolate(value, params, fmt);
	};

	formatNumber = (n: number, options?: Intl.NumberFormatOptions): string =>
		new Intl.NumberFormat(this.#intlLocale, options).format(n);

	formatDate = (date: Date | number, options: Intl.DateTimeFormatOptions = { year: 'numeric', month: 'short', day: 'numeric' }): string =>
		new Intl.DateTimeFormat(this.#intlLocale, options).format(date);

	/** "3 minutes ago", "yesterday", in the active language. */
	formatRelative = (date: Date | number, now: number = Date.now()): string => {
		const seconds = Math.round((+date - now) / 1000);
		const abs = Math.abs(seconds);
		const rtf = new Intl.RelativeTimeFormat(this.#intlLocale, { numeric: 'auto' });
		if (abs < 45) return this.t('time.justNow');
		if (abs < 3600) return rtf.format(Math.round(seconds / 60), 'minute');
		if (abs < 86400) return rtf.format(Math.round(seconds / 3600), 'hour');
		if (abs < 86400 * 30) return rtf.format(Math.round(seconds / 86400), 'day');
		return this.formatDate(date);
	};

	formatList = (items: string[], type: Intl.ListFormatType = 'conjunction'): string =>
		new Intl.ListFormat(this.#intlLocale, { style: 'long', type }).format(items);
}

export const i18n = new I18n();

/** Translate a key. Reactive inside templates and $derived. */
export const t = i18n.t;
