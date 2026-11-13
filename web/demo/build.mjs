#!/usr/bin/env node
// Builds the web client as the site's live demo into web/build.
//
// The demo routes by hash (svelte.config.js, LAIN_DEMO=1), and that router
// refuses page options. SvelteKit reads them from the source file itself,
// so the root layout's three options are taken out for this build and put
// back afterwards, whatever happens.
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const web = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const layout = path.join(web, 'src/routes/+layout.ts');
const original = fs.readFileSync(layout, 'utf8');
const stripped = original.replace(/^export const (ssr|prerender|trailingSlash) = .*\n/gm, '');
if (stripped === original) throw new Error('the root layout has no page options to take out');

let status = 1;
try {
	fs.writeFileSync(layout, stripped);
	const run = spawnSync('pnpm', ['exec', 'vite', 'build'], {
		cwd: web,
		stdio: 'inherit',
		env: { ...process.env, LAIN_DEMO: '1' }
	});
	status = run.status ?? 1;
} finally {
	fs.writeFileSync(layout, original);
}
process.exit(status);
