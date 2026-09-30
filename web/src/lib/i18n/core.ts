/*
 * Pure i18n mechanics: lookup, interpolation, plural selection, locale
 * resolution and pseudo-localization. No state and no Svelte, so every
 * rule here is unit-tested without a browser. See docs/I18N.md.
 */

/** A plural message: CLDR categories; `other` is mandatory. */
export interface PluralForms {
	zero?: string;
	one?: string;
	two?: string;
	few?: string;
	many?: string;
	other: string;
}

export type MessageValue = string | PluralForms;
export interface MessageTree {
	[key: string]: MessageValue | MessageTree;
}

export type Params = Record<string, string | number>;

/** Dotted paths to every leaf of a catalog: 'nav.home', 'settings.saved'. */
export type MessageKey<T, P extends string = ''> = {
	[K in keyof T & string]: T[K] extends string | PluralForms
		? `${P}${K}`
		: MessageKey<T[K], `${P}${K}.`>;
}[keyof T & string];

export function isPlural(value: unknown): value is PluralForms {
	return typeof value === 'object' && value !== null && typeof (value as PluralForms).other === 'string';
}

export function lookup(tree: MessageTree, key: string): MessageValue | undefined {
	let node: MessageValue | MessageTree | undefined = tree;
	for (const part of key.split('.')) {
		if (typeof node !== 'object' || node === null || isPlural(node)) return undefined;
		node = (node as MessageTree)[part];
	}
	return typeof node === 'string' || isPlural(node) ? node : undefined;
}

/** Every leaf key of a catalog, for parity checks. */
export function leafKeys(tree: MessageTree, prefix = ''): string[] {
	const out: string[] = [];
	for (const [k, v] of Object.entries(tree)) {
		const path = prefix ? `${prefix}.${k}` : k;
		if (typeof v === 'string' || isPlural(v)) out.push(path);
		else out.push(...leafKeys(v, path));
	}
	return out;
}

/** `{name}` placeholders in a template, sorted and unique. */
export function placeholders(value: MessageValue): string[] {
	const text = typeof value === 'string' ? value : Object.values(value).join(' ');
	return [...new Set([...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]))].sort();
}

/**
 * Replaces `{name}` with params. A missing param stays visible as
 * `{name}` instead of vanishing, so the gap is seen in review.
 */
export function interpolate(template: string, params: Params | undefined, formatNumber: (n: number) => string): string {
	if (!params) return template;
	return template.replace(/\{(\w+)\}/g, (whole, name: string) => {
		const v = params[name];
		if (v === undefined) return whole;
		return typeof v === 'number' ? formatNumber(v) : v;
	});
}

export function selectPlural(forms: PluralForms, count: number, rules: Intl.PluralRules): string {
	if (count === 0 && forms.zero !== undefined) return forms.zero;
	const category = rules.select(count) as keyof PluralForms;
	return forms[category] ?? forms.other;
}

/**
 * Picks the UI locale: an explicit choice, else the browser's first
 * supported language (exact tag, then its base language), else `fallback`.
 */
export function resolveLocale(
	choice: string | null | undefined,
	browser: readonly string[],
	supported: readonly string[],
	fallback: string
): string {
	if (choice && supported.includes(choice)) return choice;
	for (const tag of browser) {
		if (supported.includes(tag)) return tag;
		const base = tag.split('-')[0];
		const match = supported.find((s) => s === base || s.split('-')[0] === base);
		if (match) return match;
	}
	return fallback;
}

const ACCENTS: Record<string, string> = {
	a: 'á', b: 'ƀ', c: 'ç', d: 'ď', e: 'é', f: 'ƒ', g: 'ĝ', h: 'ĥ', i: 'í', j: 'ĵ', k: 'ķ', l: 'ľ', m: 'ɱ',
	n: 'ñ', o: 'ó', p: 'þ', q: 'ǫ', r: 'ŕ', s: 'š', t: 'ţ', u: 'ú', v: 'ṽ', w: 'ŵ', x: 'ẋ', y: 'ý', z: 'ž',
	A: 'Á', B: 'Ɓ', C: 'Ç', D: 'Ď', E: 'É', F: 'Ƒ', G: 'Ĝ', H: 'Ĥ', I: 'Í', J: 'Ĵ', K: 'Ķ', L: 'Ľ', M: 'Ṁ',
	N: 'Ñ', O: 'Ó', P: 'Þ', Q: 'Ǫ', R: 'Ŕ', S: 'Š', T: 'Ţ', U: 'Ú', V: 'Ṽ', W: 'Ŵ', X: 'Ẋ', Y: 'Ý', Z: 'Ž'
};

/**
 * Pseudo-localizes one string: accented letters, ~35% longer and
 * bracketed, placeholders untouched. Text that stays plain English in a
 * pseudo locale was never routed through `t()`; text that gets cut off
 * will break in German or Portuguese too.
 */
export function pseudo(text: string): string {
	let out = '';
	let inPlaceholder = false;
	for (const ch of text) {
		if (ch === '{') inPlaceholder = true;
		if (ch === '}') inPlaceholder = false;
		out += inPlaceholder ? ch : (ACCENTS[ch] ?? ch);
	}
	const pad = '·'.repeat(Math.max(1, Math.round(text.length * 0.35)));
	return `⟦${out}${pad}⟧`;
}

export function pseudoTree(tree: MessageTree): MessageTree {
	const out: MessageTree = {};
	for (const [k, v] of Object.entries(tree)) {
		if (typeof v === 'string') out[k] = pseudo(v);
		else if (isPlural(v)) {
			const forms: PluralForms = { other: pseudo(v.other) };
			for (const [cat, s] of Object.entries(v)) (forms as unknown as Record<string, string>)[cat] = pseudo(s as string);
			out[k] = forms;
		} else out[k] = pseudoTree(v);
	}
	return out;
}
