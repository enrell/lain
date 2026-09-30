import { describe, expect, it } from 'vitest';
import {
	interpolate,
	leafKeys,
	lookup,
	placeholders,
	pseudo,
	pseudoTree,
	resolveLocale,
	selectPlural,
	type MessageTree
} from './core';

const tree: MessageTree = {
	a: { b: 'Hello {name}', n: { one: '{count} file', other: '{count} files' } },
	top: 'Top'
};
const num = (n: number) => String(n);

describe('lookup and keys', () => {
	it('walks dotted paths to leaves only', () => {
		expect(lookup(tree, 'a.b')).toBe('Hello {name}');
		expect(lookup(tree, 'top')).toBe('Top');
		expect(lookup(tree, 'a')).toBeUndefined();
		expect(lookup(tree, 'a.b.c')).toBeUndefined();
		expect(lookup(tree, 'nope.x')).toBeUndefined();
	});
	it('lists every leaf', () => {
		expect(leafKeys(tree).sort()).toEqual(['a.b', 'a.n', 'top']);
	});
	it('collects placeholders across plural forms', () => {
		expect(placeholders('Hi {b} {a} {b}')).toEqual(['a', 'b']);
		expect(placeholders({ one: '{count} x', other: '{count} {y}' })).toEqual(['count', 'y']);
	});
});

describe('interpolate', () => {
	it('fills params and formats numbers', () => {
		expect(interpolate('Hello {name}, {n}', { name: 'Lain', n: 1200 }, (n) => n.toLocaleString('en'))).toBe('Hello Lain, 1,200');
	});
	it('keeps an unfilled placeholder visible', () => {
		expect(interpolate('Hello {name}', {}, num)).toBe('Hello {name}');
		expect(interpolate('Plain', undefined, num)).toBe('Plain');
	});
});

describe('plurals', () => {
	it('follows the language rules', () => {
		const en = new Intl.PluralRules('en');
		const forms = { one: 'one', other: 'other' };
		expect(selectPlural(forms, 1, en)).toBe('one');
		expect(selectPlural(forms, 2, en)).toBe('other');
		const ar = new Intl.PluralRules('ar');
		expect(selectPlural({ few: 'few', other: 'other' }, 3, ar)).toBe('few');
	});
	it('prefers an explicit zero form and falls back to other', () => {
		const en = new Intl.PluralRules('en');
		expect(selectPlural({ zero: 'none', other: 'n' }, 0, en)).toBe('none');
		expect(selectPlural({ other: 'n' }, 1, en)).toBe('n');
	});
});

describe('resolveLocale', () => {
	const supported = ['en', 'pt-BR', 'ja'];
	it('honours an explicit supported choice', () => {
		expect(resolveLocale('ja', ['pt-BR'], supported, 'en')).toBe('ja');
	});
	it('ignores an unsupported choice and uses the browser', () => {
		expect(resolveLocale('xx', ['pt-BR'], supported, 'en')).toBe('pt-BR');
	});
	it('matches a base language when the exact tag is missing', () => {
		expect(resolveLocale('', ['pt-PT', 'en'], supported, 'en')).toBe('pt-BR');
		expect(resolveLocale('', ['ja-JP'], supported, 'en')).toBe('ja');
	});
	it('falls back when nothing matches', () => {
		expect(resolveLocale(null, ['de-DE'], supported, 'en')).toBe('en');
	});
});

describe('pseudo', () => {
	it('accents, lengthens and brackets but keeps placeholders', () => {
		const out = pseudo('Hello {name}');
		expect(out.startsWith('⟦Ĥéľľó {name}')).toBe(true);
		expect(out.endsWith('⟧')).toBe(true);
		expect(out.length).toBeGreaterThan('Hello {name}'.length);
	});
	it('keeps the tree shape', () => {
		expect(leafKeys(pseudoTree(tree)).sort()).toEqual(leafKeys(tree).sort());
	});
});
