#!/usr/bin/env node
// Puts the live demo together:  node demo/assemble.mjs <out dir>
//
// web/build (from demo/build.mjs) + demo/sw.js + demo/snapshot  ->  <out dir>,
// ready for a static host. The page is changed in one place: the client
// starts only once the service worker that answers its API is in control,
// and it starts signed in.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const web = path.resolve(here, '..');
const out = path.resolve(process.argv[2] ?? path.join(web, 'demo-out'));
const build = path.join(web, 'build');
const snap = path.join(here, 'snapshot');
if (!fs.existsSync(path.join(build, 'index.html'))) throw new Error('no web/build: run demo/build.mjs first');
if (!fs.existsSync(path.join(snap, 'index.json'))) throw new Error('no demo/snapshot: run demo/record.mjs first');

fs.rmSync(out, { recursive: true, force: true });
fs.cpSync(build, out, { recursive: true });
fs.rmSync(path.join(out, 'service-worker.js'), { force: true }); // the offline-copies worker has no job here
fs.copyFileSync(path.join(here, 'sw.js'), path.join(out, 'sw.js'));
fs.cpSync(snap, path.join(out, 'api'), { recursive: true });

const file = path.join(out, 'index.html');
let html = fs.readFileSync(file, 'utf8');
const start = /Promise\.all\(\[\s*(import\("[^"]+"\)),\s*(import\("[^"]+"\))\s*\]\)\.then\(\(\[kit, app\]\) => \{\s*kit\.start\(app, element\);\s*\}\);/;
if (!start.test(html)) throw new Error("index.html: SvelteKit's start script is not the shape this expects");
html = html.replace(
	start,
	(_, a, b) => `
					// The live demo: no server. A service worker answers the API from a
					// recorded snapshot, so the client waits for it, then starts signed in.
					try { localStorage.getItem('lain.token') || localStorage.setItem('lain.token', 'demo'); } catch {}
					const sw = navigator.serviceWorker;
					const inControl = sw.register(new URL('sw.js', location).pathname).then(() => sw.ready).then(
						() => sw.controller || new Promise((done) => sw.addEventListener('controllerchange', done, { once: true }))
					);
					inControl.then(() => Promise.all([${a}, ${b}])).then(([kit, app]) => {
						kit.start(app, element);
					});`
);
html = html.replace('</head>', '\t<meta name="robots" content="noindex" />\n\t\t<title>lain, live demo</title>\n\t</head>');
fs.writeFileSync(file, html);
const files = fs.readdirSync(path.join(out, 'api', 'files')).length;
console.log(`demo assembled in ${out}: ${files} recorded answers`);
