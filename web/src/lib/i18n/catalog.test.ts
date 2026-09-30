/*
 * Catalog and usage checks. They run on every `npm test`, so a locale
 * that drifts from the source catalog, or code that calls t() with a
 * key that does not exist, fails CI instead of showing a raw key.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { describe, expect, it } from 'vitest';
import { isPlural, leafKeys, lookup, placeholders, type MessageTree } from './core';
import { LOCALES, SOURCE_LOCALE } from './locales';
import en from './messages/en';

const source = en as unknown as MessageTree;
const catalogs = import.meta.glob<{ default: MessageTree }>('./messages/*.ts', { eager: true });

describe('locale catalogs', () => {
	it('registers every catalog file and ships the source', () => {
		const files = Object.keys(catalogs).map((p) => p.replace('./messages/', '').replace('.ts', ''));
		expect(files).toContain(SOURCE_LOCALE);
		for (const code of files) expect(LOCALES.map((l) => l.code), code).toContain(code);
	});

	for (const [path, mod] of Object.entries(catalogs)) {
		if (path.endsWith(`/${SOURCE_LOCALE}.ts`)) continue;
		it(`${path} mirrors the source keys and placeholders`, () => {
			const tree = mod.default;
			expect(leafKeys(tree).sort()).toEqual(leafKeys(source).sort());
			for (const key of leafKeys(source)) {
				const a = lookup(source, key)!;
				const b = lookup(tree, key)!;
				expect(placeholders(b), key).toEqual(placeholders(a));
				expect(isPlural(b), key).toBe(isPlural(a));
			}
		});
	}

	it('gives every plural message an other form and a {count}', () => {
		for (const key of leafKeys(source)) {
			const v = lookup(source, key)!;
			if (isPlural(v)) expect(placeholders(v), key).toContain('count');
		}
	});
});

function sourceFiles(dir: string): string[] {
	const out: string[] = [];
	for (const name of readdirSync(dir)) {
		const p = join(dir, name);
		if (statSync(p).isDirectory()) out.push(...sourceFiles(p));
		else if (/\.(svelte|ts)$/.test(name) && !name.endsWith('.test.ts')) out.push(p);
	}
	return out;
}

describe('t() usage', () => {
	it('only asks for keys that exist', () => {
		const root = join(process.cwd(), 'src');
		const bad: string[] = [];
		for (const file of sourceFiles(root)) {
			const text = readFileSync(file, 'utf8');
			for (const m of text.matchAll(/\bt\(\s*'([a-zA-Z0-9_.]+)'/g)) {
				if (lookup(source, m[1]) === undefined) bad.push(`${relative(root, file)}: ${m[1]}`);
			}
		}
		expect(bad).toEqual([]);
	});
});
