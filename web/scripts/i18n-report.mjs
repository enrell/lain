#!/usr/bin/env node
/*
 * i18n-report — finds user-facing text that bypasses t().
 *
 *   npm run i18n:report              per-file counts, worst first
 *   npm run i18n:report -- --list    every finding with its line
 *   npm run i18n:report -- src/routes/settings   only under a path
 *
 * Heuristic, not a parser: it strips <script>/<style> and {expressions}
 * from Svelte markup, then flags text nodes and user-facing attributes
 * (aria-label, placeholder, title, alt, label, hint, description) that
 * still hold words, plus toast calls with string literals in scripts.
 * Brand and code-like tokens (Lain, Ctrl, mpv…) are ignored.
 */
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const args = process.argv.slice(2);
const list = args.includes('--list');
const roots = args.filter((a) => !a.startsWith('--'));
const base = process.cwd();
const scan = roots.length ? roots : ['src'];

const IGNORE = /^(lain|ctrl|esc|enter|shift|space|tab|mpv|vlc|hls|hdr|av1|h\.?26[45]|hevc|kbps|px|ok|id|url|api|cli)$/i;
const ATTRS = /\b(aria-label|placeholder|title|alt|label|hint|description)="([^"{}]*[A-Za-z]{2,}[^"{}]*)"/g;

function files(dir) {
	if (!statSync(dir).isDirectory()) return /\.(svelte|ts)$/.test(dir) ? [dir] : [];
	const out = [];
	for (const name of readdirSync(dir)) {
		const p = join(dir, name);
		if (statSync(p).isDirectory()) out.push(...files(p));
		else if (/\.(svelte|ts)$/.test(name) && !/\.test\.ts$/.test(name) && !p.includes('/i18n/')) out.push(p);
	}
	return out;
}

function lineOf(text, index) {
	return text.slice(0, index).split('\n').length;
}

function meaningful(s) {
	const words = s
		.trim()
		.split(/\s+/)
		.filter((w) => /[A-Za-z]{2,}/.test(w) && !/^(ctrl|shift|alt|cmd)\+\w+$/i.test(w) && !IGNORE.test(w.replace(/[^\w.]/g, '')));
	return words.length > 0;
}

function blank(text, re) {
	return text.replace(re, (m) => m.replace(/[^\n]/g, ' '));
}

const findings = [];
for (const root of scan) {
	for (const file of files(join(base, root))) {
		const raw = readFileSync(file, 'utf8');
		const rel = relative(base, file);
		if (file.endsWith('.svelte')) {
			let markup = blank(raw, /<script[\s\S]*?<\/script>/g);
			markup = blank(markup, /<style[\s\S]*?<\/style>/g);
			markup = blank(markup, /<!--[\s\S]*?-->/g);
			for (const m of markup.matchAll(ATTRS)) {
				if (meaningful(m[2])) findings.push({ file: rel, line: lineOf(markup, m.index), text: `${m[1]}="${m[2]}"` });
			}
			// Drop {…} expressions (repeat for shallow nesting), then attributes.
			let text = markup;
			for (let i = 0; i < 4; i++) text = blank(text, /\{[^{}]*\}/g);
			text = blank(text, /<[^>]*>/g);
			for (const m of text.matchAll(/[^\s][^\n]*[^\s]|[A-Za-z]{2,}/g)) {
				if (meaningful(m[0]) && /[A-Za-z]{3,}/.test(m[0])) findings.push({ file: rel, line: lineOf(text, m.index), text: m[0].trim().slice(0, 80) });
			}
		}
		const script = raw.match(/<script[\s\S]*?<\/script>/)?.[0] ?? (file.endsWith('.ts') ? raw : '');
		const offset = raw.indexOf(script);
		for (const m of script.matchAll(/toasts\.(?:success|error|info)\(\s*(['`"])([^'`"]*[A-Za-z]{3,}[^'`"]*)\1/g)) {
			findings.push({ file: rel, line: lineOf(raw, offset + m.index), text: `toast: ${m[2].slice(0, 70)}` });
		}
	}
}

const byFile = new Map();
for (const f of findings) byFile.set(f.file, (byFile.get(f.file) ?? 0) + 1);
const ranked = [...byFile.entries()].sort((a, b) => b[1] - a[1]);

if (list) {
	for (const f of findings) console.log(`${f.file}:${f.line}  ${f.text}`);
} else {
	for (const [file, n] of ranked) console.log(`${String(n).padStart(5)}  ${file}`);
}
console.log(`\n${findings.length} hard-coded strings in ${byFile.size} files (heuristic; see docs/I18N.md).`);
