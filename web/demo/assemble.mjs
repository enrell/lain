#!/usr/bin/env node
// Puts the live demo together:  node demo/assemble.mjs <out dir>
//
// web/build (from demo/build.mjs) + demo/sw.js + demo/snapshot  ->  <out dir>,
// ready for a static host. The page is changed in one place: the client
// starts only once the service worker that answers its API is in control,
// and it starts signed in.
import crypto from 'node:crypto';
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
const snapHash = crypto
	.createHash('sha256')
	.update(fs.readFileSync(path.join(snap, 'index.json')))
	.digest('hex')
	.slice(0, 16);
const swSource = fs.readFileSync(path.join(here, 'sw.js'), 'utf8').replace('__SNAPSHOT_VERSION__', snapHash);
fs.writeFileSync(path.join(out, 'sw.js'), swSource);
fs.cpSync(snap, path.join(out, 'api'), { recursive: true });

const file = path.join(out, 'index.html');
let html = fs.readFileSync(file, 'utf8');
// Make index.html relocatable across any mount path (/demo/ locally, /lain/demo/ on GitHub Pages)
// by resolving asset references relative to the document directory instead of the compile-time base.
const builtBase = (html.match(/\bassets:\s*(["'])([^"']*)\1/)?.[2] ?? process.env.LAIN_DEMO_BASE ?? '/lain/demo').replace(/\/$/, '');
const prefixPattern = builtBase.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
html = html
	.replace(/\bassets:\s*(["'])[^"']*\1/, "assets: new URL('.', location).pathname.slice(0, -1)")
	.replace(new RegExp(`(href=|import\\()(["'])${prefixPattern}/(?!/)`, 'g'), '$1$2./');
const start = /Promise\.all\(\[\s*(import\("[^"]+"\)),\s*(import\("[^"]+"\))\s*\]\)\.then\(\(\[kit, app\]\) => \{\s*kit\.start\(app, element\);\s*\}\);/;
if (!start.test(html)) throw new Error("index.html: SvelteKit's start script is not the shape this expects");
html = html.replace(
	start,
	(_, a, b) => `
					// The live demo: no server. A service worker answers the API from a
					// recorded snapshot, so the client waits for it, then starts signed in.
					try { localStorage.getItem('lain.token') || localStorage.setItem('lain.token', 'demo'); } catch {}
					const sw = navigator.serviceWorker;
					const awaitWorker = (worker, reg) =>
						new Promise((done) => {
							if (!worker || sw.controller === worker) return done();
							const finish = () => {
								sw.removeEventListener('controllerchange', onChange);
								done();
							};
							const onChange = () => {
								if (sw.controller === worker || (!reg.installing && !reg.waiting)) finish();
							};
							sw.addEventListener('controllerchange', onChange);
							worker.addEventListener('statechange', () => {
								if (sw.controller === worker || (worker.state === 'redundant' && !reg.installing && !reg.waiting)) finish();
							});
							setTimeout(finish, 3000);
						});
					const inControl = sw
						.register(new URL('sw.js', location).pathname, { updateViaCache: 'none' })
						.then(async (reg) => {
							let pending = reg.installing || reg.waiting;
							if (!pending) {
								await reg.update().catch(() => {});
								pending = reg.installing || reg.waiting;
							}
							if (pending) {
								await awaitWorker(pending, reg);
								const newer = reg.installing || reg.waiting;
								if (newer) await awaitWorker(newer, reg);
							}
							await sw.ready;
							return sw.controller || new Promise((done) => sw.addEventListener('controllerchange', done, { once: true }));
						});
					inControl.then(() => Promise.all([${a}, ${b}])).then(([kit, app]) => {
						kit.start(app, element);
					});`
);
html = html.replace('</head>', '\t<meta name="robots" content="noindex" />\n\t\t<title>lain, live demo</title>\n\t</head>');
fs.writeFileSync(file, html);
const files = fs.readdirSync(path.join(out, 'api', 'files')).length;
console.log(`demo assembled in ${out}: ${files} recorded answers`);
