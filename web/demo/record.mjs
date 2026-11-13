#!/usr/bin/env node
// Records the demo's snapshot: walks the client in a headless browser while
// demo/serve.mjs proxies and records the API it asks for.
//
//   node demo/serve.mjs --backend http://127.0.0.1:9360 --record demo/snapshot &
//   PLAYWRIGHT_CORE=/path/to/playwright-core/index.mjs \
//     node demo/record.mjs --user NAME --password PASSWORD
//
// The server it records should hold only what may be published: the
// snapshot (catalog, pictures, comic pages) is committed and served.
import fs from 'node:fs';

const args = Object.fromEntries(
	process.argv.slice(2).reduce((out, a, i, all) => (a.startsWith('--') ? [...out, [a.slice(2), all[i + 1] ?? '']] : out), [])
);
const origin = args.origin ?? 'http://127.0.0.1:5300';
const base = args.base ?? '/lain/demo';
const { chromium } = await import(process.env.PLAYWRIGHT_CORE ?? 'playwright-core');

const api = async (path, init) => (await fetch(origin + path, init)).json();
const login = await api('/api/auth/login', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ username: args.user, password: args.password }) });
const auth = { headers: { authorization: `Bearer ${login.token}` } };
const libraries = await api('/api/libraries', auth);
const all = await api('/api/catalog?group=title&limit=500', auth); // also what the demo's search reads
const titles = new Map();
for (const item of all.items) if (!titles.has(`${item.library_id}|${item.title}`)) titles.set(`${item.library_id}|${item.title}`, item);
const books = all.items.filter((i) => i.kind === 'manga' || i.kind === 'comic' || i.kind === 'book');
const videos = [...titles.values()].filter((i) => !books.includes(i));

const routes = [
	'/', '/library?view=shelves', '/library?view=all', '/search', '/search?q=a', '/favorites', '/list', '/social', '/acquire',
	...libraries.map((l) => `/library/${l.id}`),
	...[...titles.values()].map((i) => `/item/${i.id}`),
	...videos.map((i) => `/player/${i.id}`),
	...['profile', 'playback', 'connections', 'security', 'privacy', 'offline', 'libraries', 'users', 'transcoding', 'integrations', 'plugins', 'backup', 'downloads', 'indexers', 'acquisition'].map((s) => `/settings/${s}`)
];

const browser = await chromium.launch({ executablePath: process.env.CHROME ?? '/usr/bin/chromium', headless: true, args: ['--no-sandbox', '--mute-audio'] });
for (const [name, viewport, extra] of [
	['desktop', { width: 1440, height: 900 }, {}],
	['phone', { width: 390, height: 844 }, { hasTouch: true, isMobile: true, deviceScaleFactor: 2 }]
]) {
	const context = await browser.newContext({ viewport, ...extra });
	await context.addInitScript((t) => localStorage.setItem('lain.token', t), login.token);
	const page = await context.newPage();
	const visit = async (route) => {
		await page.goto(`${origin}${base}/#${route}`, { waitUntil: 'networkidle', timeout: 45000 }).catch(() => {});
		await page.waitForTimeout(900);
		// Rows and rails ask for their pictures as they come into view.
		await page.evaluate(async () => {
			for (let y = 0; y < document.documentElement.scrollHeight; y += 600) {
				window.scrollTo(0, y);
				await new Promise((r) => setTimeout(r, 120));
			}
			window.scrollTo(0, 0);
		});
		await page.waitForLoadState('networkidle', { timeout: 15000 }).catch(() => {});
	};
	for (const route of routes) await visit(route);
	// Tabs that load on demand.
	for (const [route, labels] of [['/social', ['Friends', 'Notifications', 'Collections']], ['/acquire', ['Queue', 'Wanted', 'Subtitles', 'Settings']]]) {
		await visit(route);
		for (const label of labels) {
			await page.getByRole('tab', { name: label }).first().click({ timeout: 2000 }).catch(() => {});
			await page.waitForTimeout(700);
		}
	}
	// Every page of every book, one page at a time.
	for (const book of books) {
		await visit(`/read/${book.id}`);
		for (let i = 0; i < 16; i++) {
			await page.keyboard.press('ArrowLeft');
			await page.waitForTimeout(250);
		}
		for (let i = 0; i < 32; i++) {
			await page.keyboard.press('ArrowRight');
			await page.waitForTimeout(250);
		}
	}
	await context.close();
	console.log(`${name}: ${routes.length} routes, ${books.length} books`);
}
await browser.close();
