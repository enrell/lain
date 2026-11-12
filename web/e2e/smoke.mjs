#!/usr/bin/env node
/*
 * Lain browser smoke test.
 *
 * Drives a real Chromium over the DevTools Protocol using only Node
 * built-ins (global fetch + WebSocket) against a *fresh* Lain data
 * directory. It exercises the actual lain server (the release binary
 * with the web build embedded) with a real database and real media
 * files:
 *
 *   setup -> libraries -> scan -> browse -> header search -> search
 *   -> item enrichment -> favorites -> title page -> HLS, progressive,
 *   far seek, direct Matroska and frame-proof fallback playback
 *   -> Range -> keyboard seek -> bounded progress writes -> player
 *   chrome -> subtitles -> continue watching -> admin surfaces
 *   -> swap fencing -> backup -> non-admin gating -> deep links
 *   -> responsive layouts -> quality gates
 *
 * Usage: web/e2e/run-smoke.sh does all of it (fixtures, a fresh server,
 * this test, the server stopped); CI runs that. By hand:
 *   web/e2e/fixtures.sh /tmp/lain-fixtures          # synthetic media
 *   lain serve --data-dir /tmp/lain-e2e --port 9360 # fresh dir
 *     (on a machine with network, point LAIN_METADATA_KITSU_URL,
 *     _ANILIST_URL, _JIKAN_URL and _TVMAZE_URL at a closed port, e.g.
 *     http://127.0.0.1:9, so the metadata step meets the local NFO
 *     provider: online answers would win over it)
 *   node web/e2e/smoke.mjs \
 *     --base http://127.0.0.1:9360 \
 *     --media /tmp/lain-fixtures
 *
 * The browser is --chromium PATH, else $CHROME, else the first of
 * chromium, chromium-browser, google-chrome-stable and google-chrome on
 * PATH (H.264 and AAC must decode: Google Chrome, or a distribution
 * Chromium built with them). $CHROME_FLAGS adds browser flags (CI
 * passes --no-sandbox). --transcode-timeout MS bounds every wait on
 * ffmpeg (default 90000).
 *
 * Exits non-zero when any assertion fails or the console/network
 * quality gate trips. Requires Node >= 22 (built-in WebSocket).
 */
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { verifyPlayerControls } from './player-controls.mjs';

const args = process.argv.slice(2);
const arg = (name, fallback) => {
	const i = args.indexOf(`--${name}`);
	return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};

/** The first Chromium-family browser on PATH, for when none is named. */
function findChrome() {
	const names = ['chromium', 'chromium-browser', 'google-chrome-stable', 'google-chrome'];
	for (const dir of (process.env.PATH ?? '').split(':').filter(Boolean)) {
		for (const name of names) {
			if (existsSync(join(dir, name))) return join(dir, name);
		}
	}
	return 'chromium';
}

const BASE = arg('base', 'http://127.0.0.1:9360');
const MEDIA = arg('media', '/tmp/lain-e2e-fixtures');
const CHROMIUM = arg('chromium', process.env.CHROME || findChrome());
const PORT = Number(arg('debug-port', '9333'));
const PROFILE = arg('profile', '/tmp/lain-e2e-chrome');
const DOWNLOADS = arg('downloads', '/tmp/lain-e2e-downloads');
const SCREENSHOT = arg('screenshot', '/tmp/lain-e2e-failure.png');
const ADMIN_USER = arg('user', 'admin');
const ADMIN_PASS = arg('pass', 'password123');
const NORMAL_USER = arg('user2', 'ana');
const NORMAL_PASS = arg('pass2', 'password123');

// The bound on every wait that runs through ffmpeg (a session starting,
// its playlist, segments, a rebuild, the fallback's first picture). A
// shared CI runner encodes at a fraction of a workstation's speed; the
// waits end as soon as their condition holds, so a generous bound costs
// nothing on a fast machine and only turns a stall into a failure.
const TRANSCODE_MS = Number(arg('transcode-timeout', '90000'));

function assert(cond, message) {
	if (!cond) throw new Error(message);
}

// Each step says when it started: on a slow runner the log alone tells
// which step ate the time.
const startedAt = Date.now();
function step(name) {
	const at = ((Date.now() - startedAt) / 1000).toFixed(1);
	process.stdout.write(`\n== ${name} (+${at}s)\n`);
}

/* ------------------------------------------------------------------ */
/* chromium + CDP                                                      */
/* ------------------------------------------------------------------ */

rmSync(PROFILE, { recursive: true, force: true });
rmSync(DOWNLOADS, { recursive: true, force: true });
mkdirSync(DOWNLOADS, { recursive: true });

const chrome = spawn(
	CHROMIUM,
	[
		'--headless=new',
		'--disable-gpu',
		'--no-first-run',
		'--no-default-browser-check',
		'--disable-dev-shm-usage',
		'--mute-audio',
		'--autoplay-policy=no-user-gesture-required',
		// A packaged Chrome loads the extensions its distribution drops in
		// /usr/share (a fresh profile does not stop them), and one that
		// opens its own tab sends this one to the background, where
		// requestAnimationFrame never fires and every settle() hangs.
		'--disable-extensions',
		'--disable-component-extensions-with-background-pages',
		'--disable-default-apps',
		'--disable-sync',
		'--disable-background-networking',
		// The test's tab must keep its timers and frames even if something
		// else takes the front: the player's idle fade and the frame proof
		// are timers.
		'--disable-background-timer-throttling',
		'--disable-backgrounding-occluded-windows',
		'--disable-renderer-backgrounding',
		'--password-store=basic',
		...(process.env.CHROME_FLAGS ?? '').split(/\s+/).filter(Boolean),
		`--remote-debugging-port=${PORT}`,
		`--user-data-dir=${PROFILE}`,
		'about:blank'
	],
	{ stdio: ['ignore', 'ignore', 'ignore'] }
);
chrome.on('error', (err) => {
	console.error('failed to launch chromium:', err.message);
	process.exit(2);
});

async function waitForDevtools() {
	const deadline = Date.now() + 15000;
	while (Date.now() < deadline) {
		try {
			const res = await fetch(`http://127.0.0.1:${PORT}/json/version`);
			if (res.ok) return;
		} catch {
			/* not up yet */
		}
		await sleep(150);
	}
	throw new Error('chromium devtools endpoint did not come up');
}

class CDP {
	constructor(ws) {
		this.ws = ws;
		this.id = 0;
		this.pending = new Map();
		this.listeners = new Map();
		ws.addEventListener('message', (event) => {
			const msg = JSON.parse(event.data);
			if (msg.id !== undefined) {
				const entry = this.pending.get(msg.id);
				if (!entry) return;
				this.pending.delete(msg.id);
				if (msg.error) entry.reject(new Error(`${msg.error.message} (${msg.error.code})`));
				else entry.resolve(msg.result);
				return;
			}
			for (const fn of this.listeners.get(msg.method) ?? []) fn(msg.params);
		});
	}

	static async connect(url) {
		const ws = new WebSocket(url);
		await new Promise((resolve, reject) => {
			ws.addEventListener('open', resolve, { once: true });
			ws.addEventListener('error', () => reject(new Error('websocket failed: ' + url)), {
				once: true
			});
		});
		return new CDP(ws);
	}

	/**
	 * Every call is bounded: an evaluation the page never answers (a
	 * backgrounded tab's requestAnimationFrame, a modal JS dialog) fails
	 * the run with its name instead of holding it until the job times out.
	 */
	send(method, params = {}, timeoutMs = 60000) {
		const id = ++this.id;
		return new Promise((resolve, reject) => {
			const timer = setTimeout(() => {
				this.pending.delete(id);
				reject(new Error(`CDP ${method} did not answer in ${timeoutMs}ms`));
			}, timeoutMs);
			this.pending.set(id, {
				resolve: (v) => {
					clearTimeout(timer);
					resolve(v);
				},
				reject: (e) => {
					clearTimeout(timer);
					reject(e);
				}
			});
			this.ws.send(JSON.stringify({ id, method, params }));
		});
	}

	on(method, fn) {
		if (!this.listeners.has(method)) this.listeners.set(method, []);
		this.listeners.get(method).push(fn);
	}

	close() {
		this.ws.close();
	}
}

/* ------------------------------------------------------------------ */
/* state + helpers                                                     */
/* ------------------------------------------------------------------ */

let page;
let browser;
const requests = []; // {requestId, url, method, headers, status}
const consoleErrors = [];
const allConsole = [];
const seen404 = new Set();
const playlists = []; // the last HLS playlists served, for a failure report
const requestCount = new Map();

function waitUntil(condition, timeoutMs = 5000, label = 'condition') {
	const deadline = Date.now() + timeoutMs;
	return new Promise((resolve, reject) => {
		const tick = () => {
			if (condition()) return resolve(true);
			if (Date.now() > deadline) return reject(new Error('timed out waiting for ' + label));
			setTimeout(tick, 100);
		};
		tick();
	});
}

async function evalValue(expression) {
	const { result, exceptionDetails } = await page.send('Runtime.evaluate', {
		expression,
		returnByValue: true,
		awaitPromise: true
	});
	if (exceptionDetails) {
		throw new Error(
			'page exception: ' + (exceptionDetails.exception?.description ?? exceptionDetails.text)
		);
	}
	return result.value;
}

async function settle() {
	// Two frames, or half a second where frames are not being produced
	// (a tab that lost the front): a settle is a courtesy, never a wait.
	await evalValue(
		'new Promise((resolve) => { requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))); setTimeout(() => resolve(false), 500); })'
	);
}

async function waitText(text, timeoutMs = 15000) {
	const deadline = Date.now() + timeoutMs;
	// The UI's copy uses typographic apostrophes (’) and the test's literals
	// straight ones; either way it is the same text.
	const wanted = JSON.stringify(text.replace(/[‘’]/g, "'"));
	while (Date.now() < deadline) {
		const found = await evalValue(
			`document.body ? document.body.innerText.replace(/[\\u2018\\u2019]/g, "'").includes(${wanted}) : false`
		);
		if (found) return;
		await sleep(120);
	}
	const body = await evalValue('document.body ? document.body.innerText.slice(0, 600) : ""');
	throw new Error(`text not found: ${JSON.stringify(text)}\n--- page ---\n${body}`);
}

async function waitGoneText(text, timeoutMs = 15000) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		const found = await evalValue(
			`document.body ? document.body.innerText.includes(${JSON.stringify(text)}) : false`
		);
		if (!found) return;
		await sleep(120);
	}
	throw new Error('text did not disappear: ' + text);
}

async function waitFor(expression, timeoutMs = 15000, label = expression) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		if (await evalValue(`!!(${expression})`)) return;
		await sleep(120);
	}
	throw new Error('condition not met: ' + label);
}

/** Real trusted click at the element's center, retried while a dialog
 * overlay finishes its exit transition. */
async function openPlayerSettings() {
	const y = await evalValue(`document.querySelector('footer').getBoundingClientRect().top + 8`);
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 100, y });
	await waitFor(`getComputedStyle(document.querySelector('footer')).opacity === '1'`, 5000, 'controls revealed');
	await clickText('Playback settings', 'button[aria-label="Playback settings"]');
}

async function clickText(text, selector = 'button, a') {
	await settle();
	const deadline = Date.now() + 5000;
	let info = null;
	let prev = null;
	while (Date.now() < deadline) {
		info = await evalValue(`(() => {
			const els = [...document.querySelectorAll(${JSON.stringify(selector)})];
			const el = els.find((e) => (e.textContent || '').trim().includes(${JSON.stringify(text)}));
			if (!el) return null;
			el.scrollIntoView({ block: 'center', inline: 'center' });
			const rect = el.getBoundingClientRect();
			const x = rect.left + rect.width / 2;
			const y = rect.top + rect.height / 2;
			const hit = document.elementFromPoint(x, y);
			// What is hit must be the element looked for: a link or button, or whatever the selector names (a menu item is neither).
			const hitEl = hit ? (hit.closest(${JSON.stringify(selector)}) ?? hit.closest('a, button')) : null;
			return {
				tag: el.tagName,
				href: el.getAttribute('href'),
				x,
				y,
				hit: hitEl ? (hitEl.textContent || '').trim().slice(0, 60) : (hit ? hit.tagName : null)
			};
		})()`);
		// Twice at the same point: scrolling an element into view moves a
		// floating menu a frame later, and a click measured in between lands
		// where the entry no longer is.
		const steady = info && prev && prev.x === info.x && prev.y === info.y;
		if (info && info.hit && info.hit.includes(text) && steady) break;
		prev = info;
		await sleep(100);
	}
	assert(info, `no clickable element with text ${JSON.stringify(text)}`);
	assert(
		info.hit && info.hit.includes(text),
		`click point for ${JSON.stringify(text)} is covered by ${JSON.stringify(info.hit)}`
	);
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: info.x, y: info.y });
	for (const type of ['mousePressed', 'mouseReleased']) {
		await page.send('Input.dispatchMouseEvent', {
			type,
			x: info.x,
			y: info.y,
			button: 'left',
			clickCount: 1
		});
	}
	console.log(`   click ${text} -> ${info.tag}${info.href ? ' ' + info.href : ''}`);
}

/** Real trusted click at the centre of the element a selector names. */
async function clickSelector(selector, label = selector) {
	await settle();
	const point = await evalValue(`(() => {
		const el = document.querySelector(${JSON.stringify(selector)});
		if (!el) return null;
		el.scrollIntoView({ block: 'center', inline: 'center' });
		const r = el.getBoundingClientRect();
		const x = r.left + r.width / 2;
		const y = r.top + r.height / 2;
		const hit = document.elementFromPoint(x, y);
		return { x, y, hit: !!hit && (hit === el || el.contains(hit)) };
	})()`);
	assert(point, `nothing matches ${label}`);
	assert(point.hit, `the click point of ${label} is covered`);
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: point.x, y: point.y });
	for (const type of ['mousePressed', 'mouseReleased']) {
		await page.send('Input.dispatchMouseEvent', {
			type,
			x: point.x,
			y: point.y,
			button: 'left',
			clickCount: 1
		});
	}
	console.log(`   click ${label}`);
}

async function fillLabel(labelText, value) {
	const filled = await evalValue(`(() => {
		const labels = [...document.querySelectorAll('label')];
		const label = labels.find((l) => (l.textContent || '').trim().toLowerCase().startsWith(${JSON.stringify(labelText.toLowerCase())}));
		if (!label || !label.htmlFor) return false;
		const el = document.getElementById(label.htmlFor);
		if (!el) return false;
		const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
		const setter = Object.getOwnPropertyDescriptor(proto, 'value').set;
		setter.call(el, ${JSON.stringify(value)});
		el.dispatchEvent(new Event('input', { bubbles: true }));
		el.dispatchEvent(new Event('change', { bubbles: true }));
		return true;
	})()`);
	assert(filled, `no input for label ${JSON.stringify(labelText)}`);
}

// The add-library dialog has one path bar: a typed path is opened when the
// field is left (or on Enter), and the Add buttons wait for the server to
// have listed that folder.
async function openFolder(path) {
	// The dialog lists its starting folder as it opens, and that answer can
	// land after the typed path's: the path is typed again until it holds.
	const opened = `[...document.querySelectorAll('button')].some((b) => (b.textContent || '').includes('Add and scan') && !b.disabled) && document.getElementById('library-path').value === ${JSON.stringify(path)}`;
	const deadline = Date.now() + 10000;
	while (Date.now() < deadline) {
		await fillLabel('Folder', path);
		await evalValue(`document.getElementById('library-path').dispatchEvent(new FocusEvent('blur'))`);
		for (let i = 0; i < 8; i++) {
			if (await evalValue(opened)) return;
			await sleep(120);
		}
	}
	assert(false, 'the typed folder was not opened');
}

async function fillPlaceholder(placeholder, value) {
	const filled = await evalValue(`(() => {
		const el = document.querySelector('input[placeholder=' + ${JSON.stringify(JSON.stringify(placeholder))} + ']');
		if (!el) return false;
		el.focus();
		const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
		setter.call(el, ${JSON.stringify(value)});
		el.dispatchEvent(new Event('input', { bubbles: true }));
		el.dispatchEvent(new Event('change', { bubbles: true }));
		return true;
	})()`);
	assert(filled, `no input with placeholder ${JSON.stringify(placeholder)}`);
}

/**
 * Type into the search bar the screen shows: the header's field from md
 * up (it drives /search's results), the page's own field on a phone.
 */
async function fillSearch(value) {
	const filled = await evalValue(`(() => {
		const shown = (el) => el && el.getClientRects().length > 0;
		const header = document.getElementById('header-search');
		const el = shown(header) ? header : document.querySelector('input[placeholder="Titles, not filenames"]');
		if (!shown(el)) return false;
		el.focus();
		const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
		setter.call(el, ${JSON.stringify(value)});
		el.dispatchEvent(new Event('input', { bubbles: true }));
		return true;
	})()`);
	assert(filled, 'no search field on screen');
}

/** Set a native <select> (bound with Svelte bind:value) and fire change. */
async function selectOption(ariaLabel, value) {
	const ok = await evalValue(`(() => {
		const el = document.querySelector('select[aria-label=' + ${JSON.stringify(JSON.stringify(ariaLabel))} + ']');
		if (!el) return false;
		el.value = ${JSON.stringify(value)};
		el.dispatchEvent(new Event('change', { bubbles: true }));
		return el.value === ${JSON.stringify(value)};
	})()`);
	assert(ok, `could not set select ${JSON.stringify(ariaLabel)} to ${JSON.stringify(value)}`);
}

async function pressKey(key, code, keyCode) {
	for (const type of ['keyDown', 'keyUp']) {
		await page.send('Input.dispatchKeyEvent', {
			type,
			key,
			code,
			windowsVirtualKeyCode: keyCode,
			nativeVirtualKeyCode: keyCode
		});
	}
}

async function navigate(url) {
	requests.length = 0;
	requestCount.clear();
	await page.send('Page.navigate', { url });
	await waitFor(`document.readyState === 'complete'`, 20000, 'document load');
}

/* ------------------------------------------------------------------ */
/* main                                                                */
/* ------------------------------------------------------------------ */

const evidence = {};
let exitCode = 0;
try {
	await waitForDevtools();
	const version = await (await fetch(`http://127.0.0.1:${PORT}/json/version`)).json();
	console.log(`browser: ${version['Browser']} (${CHROMIUM})`);

	const target = await (
		await fetch(`http://127.0.0.1:${PORT}/json/new?${encodeURIComponent(BASE + '/')}`, {
			method: 'PUT'
		})
	).json();
	page = await CDP.connect(target.webSocketDebuggerUrl);
	browser = await CDP.connect(version.webSocketDebuggerUrl);

	await page.send('Page.enable');
	await page.send('Page.bringToFront');
	await page.send('Runtime.enable');
	await page.send('Network.enable');
	await page.send('Log.enable');
	await browser.send('Browser.setDownloadBehavior', {
		behavior: 'allow',
		downloadPath: DOWNLOADS,
		eventsEnabled: true
	});

	page.on('Network.requestWillBeSent', (e) => {
		requests.push({
			requestId: e.requestId,
			url: e.request.url,
			method: e.request.method,
			headers: e.request.headers,
			// POST bodies matter for the transcode session: the rebuilt
			// session's start_sec is the proof that a far seek started
			// producing at the target instead of clamping to the edge.
			postData: e.request.postData ?? null,
			// Timestamps turn a seek into a timeline: the wait is made of
			// polling, playlist freshness and the player attaching, and only
			// measuring tells which one dominates.
			ts: Date.now(),
			status: null
		});
		const key = e.request.method + ' ' + e.request.url;
		requestCount.set(key, (requestCount.get(key) ?? 0) + 1);
	});
	page.on('Network.responseReceived', (e) => {
		const last = [...requests].reverse().find((r) => r.url === e.response.url && r.status === null);
		if (last) last.status = e.response.status;
		if (e.response.status === 404) seen404.add(e.response.url);
	});
	// The last few HLS playlists as served: when a transcode step fails,
	// what the player was handed is the first thing to know.
	page.on('Network.loadingFinished', (e) => {
		const r = requests.find((x) => x.requestId === e.requestId);
		if (!r || !r.url.includes('.m3u8')) return;
		page
			.send('Network.getResponseBody', { requestId: e.requestId })
			.then((b) => {
				const text = b.base64Encoded ? Buffer.from(b.body, 'base64').toString() : b.body;
				playlists.push({ at: Date.now(), url: r.url.replace(BASE, '').replace(/token=[^&]+/, 'token=…'), text });
				if (playlists.length > 4) playlists.shift();
			})
			.catch(() => {});
	});
	page.on('Page.navigatedWithinDocument', (e) => {
		allConsole.push(`history: ${e.url.replace(BASE, '')}`);
	});
	page.on('Runtime.exceptionThrown', (e) => {
		consoleErrors.push(
			'exception: ' + (e.exceptionDetails?.exception?.description ?? e.exceptionDetails?.text)
		);
	});
	page.on('Runtime.consoleAPICalled', (e) => {
		const text = e.args.map((a) => a.value ?? a.description ?? '').join(' ');
		allConsole.push(`${e.type}: ${text}`);
		if (e.type === 'error') consoleErrors.push('console.error: ' + text);
	});
	page.on('Log.entryAdded', (e) => {
		if (e.entry.level === 'error') consoleErrors.push(`log[${e.entry.source}]: ${e.entry.text}`);
	});

	/* ---------------- setup ---------------- */
	step('first-run setup');
	await waitText('Create your account');
	const publicTheme = await evalValue(`fetch('/api/theme').then((r) => r.json())`);
	await waitUntil(
		() => requests.some((r) => r.url.endsWith('/api/theme') && r.status === 200),
		5000,
		'public theme request'
	);
	await waitFor(
		`getComputedStyle(document.documentElement).getPropertyValue('--color-accent').trim() === ${JSON.stringify(publicTheme.accent)}`,
		5000,
		'Omarchy theme token'
	);
	await fillLabel('Username', ADMIN_USER);
	await fillLabel('Password', ADMIN_PASS);
	await fillLabel('Confirm password', ADMIN_PASS);
	await clickText('Create account');
	await waitText("You're in.");
	// The welcome step lands on its one next step, so Enter takes it.
	await waitFor(
		`document.activeElement?.id === 'setup-add-library'`,
		5000,
		'focus on Add a media library after setup'
	);
	console.log('   setup account created and authenticated');

	step('create and scan libraries');
	// Add a media library opens Libraries with its Add dialog already open.
	const addDialog = `document.querySelector('[role="dialog"] #library-path')`;
	await clickText('Add a media library');
	await waitFor(`location.pathname === '/settings/libraries'`, 10000, 'settings/libraries URL');
	await waitText('Media libraries', 10000);
	await waitFor(addDialog, 5000, 'the Add dialog opened from setup');
	// The empty state's own button opens it too: Esc closes it, the button
	// brings it back.
	await pressKey('Escape', 'Escape', 27);
	await waitFor(`!${addDialog}`, 5000, 'Esc closed the Add dialog');
	await clickText('Add your first library');
	await waitFor(addDialog, 5000, 'Add your first library opened the Add dialog');
	await openFolder(`${MEDIA}/Anime`);
	await fillLabel('Name', 'Anime');
	await clickText('Add and scan');
	await waitFor(`document.body.innerText.includes('identified')`, 20000, 'first scan finished');

	await clickText('Add library');
	await waitText('Add a media library', 5000);
	await openFolder(`${MEDIA}/Movies`);
	await fillLabel('Name', 'Movies');
	await clickText('Add and scan');
	await waitFor(`document.body.innerText.includes('identified')`, 20000, 'second scan finished');
	console.log('   two libraries scanned');

	// Home has drawn its rows: one per library, its newest titles (D-283).
	const homeRows = () =>
		waitFor(
			`[...document.querySelectorAll('section h2')].some((h) => h.textContent.trim() === 'Movies')`,
			15000,
			'Home\'s row for the Movies library'
		);

	/* ---------------- browse ---------------- */
	step('home, catalog and search');
	await clickText('Home');
	await homeRows();
	await waitFor(
		`[...document.querySelectorAll('a[href^="/item/"]')].length >= 3`,
		15000,
		'the libraries\' rows of titles'
	);
	const cards = await evalValue(`document.querySelectorAll('a[href^="/item/"]').length`);
	console.log(`   ${cards} media cards on Home`);

	/* ---------------- the header's search field ---------------- */
	// The header holds the search field itself (no Search link): `/` puts
	// the cursor in it from anywhere, typing offers the top titles under
	// it with "All results" first, arrows pick a row and Enter opens it.
	// Typed with real key events, so the field's own handlers are what
	// is exercised, not a value set from outside.
	step('the header search field suggests titles; Enter opens results or a title');
	{
		await page.send('Emulation.setDeviceMetricsOverride', {
			width: 1440,
			height: 900,
			deviceScaleFactor: 1,
			mobile: false
		});
		const field = `document.getElementById('header-search')`;
		const options = `[...document.querySelectorAll('#header-search-list [role="option"]')]`;
		const typeInHeader = async (text) => {
			await navigate(`${BASE}/`);
			await homeRows();
			await page.send('Page.bringToFront');
			await evalValue('document.activeElement?.blur?.()');
			await pressKey('/', 'Slash', 191);
			await waitFor(`document.activeElement === ${field}`, 5000, '/ focused the header search field');
			await page.send('Input.insertText', { text });
			await waitFor(
				`${field}.getAttribute('aria-expanded') === 'true' && ${options}.some((o) => o.id.startsWith('header-search-title-') && o.textContent.toLowerCase().includes('frieren'))`,
				10000,
				'the suggestion list offers the title'
			);
		};
		await typeInHeader('frie');
		const rows = await evalValue(
			`${options}.map((o) => ({ id: o.id, selected: o.getAttribute('aria-selected') }))`
		);
		assert(
			rows[0]?.id === 'header-search-all' && rows[0].selected === 'true',
			'"All results" is not the first, chosen row: ' + JSON.stringify(rows)
		);
		await pressKey('ArrowDown', 'ArrowDown', 40);
		await waitFor(
			`${field}.getAttribute('aria-activedescendant')?.startsWith('header-search-title-')`,
			3000,
			'ArrowDown picked a title'
		);
		await pressKey('ArrowUp', 'ArrowUp', 38);
		await waitFor(
			`${field}.getAttribute('aria-activedescendant') === 'header-search-all'`,
			3000,
			'ArrowUp went back to every result'
		);
		await pressKey('Enter', 'Enter', 13);
		await waitFor(
			`location.pathname === '/search' && new URLSearchParams(location.search).get('q') === 'frie'`,
			10000,
			'Enter opened /search?q=frie'
		);
		await waitFor(`document.body.innerText.includes('1 title for')`, 15000, 'results for the typed query');
		// On /search the header field is the page's: it keeps the query.
		assert(
			(await evalValue(`${field}.value`)) === 'frie',
			'the header field lost the query on /search'
		);

		// A title row opens the title page, and the field goes back to rest.
		await typeInHeader('frie');
		await pressKey('ArrowDown', 'ArrowDown', 40);
		await pressKey('Enter', 'Enter', 13);
		await waitFor(`location.pathname.startsWith('/item/')`, 10000, 'Enter on a title opened it');
		await waitFor(`${field}.value === '' && document.activeElement !== ${field}`, 5000, 'the field went back to rest');

		// Escape clears a query first, then leaves the field.
		await typeInHeader('frie');
		await pressKey('Escape', 'Escape', 27);
		await waitFor(`${field}.value === '' && document.activeElement === ${field}`, 3000, 'Escape cleared the field');
		await pressKey('Escape', 'Escape', 27);
		await waitFor(`document.activeElement !== ${field}`, 3000, 'a second Escape left the field');

		// On /search the field writes what is typed to the URL a moment after
		// the last key. A link followed inside that moment must win: the
		// pending write used to replace it with /search again. The click
		// lands well inside the 250ms debounce, and Home must still be where
		// the viewer is a second later, four debounces on.
		await navigate(`${BASE}/search?q=frie`);
		await waitFor(`${field}.value === 'frie' && document.body.innerText.includes('1 title for')`, 15000, 'the field holds the page query');
		await evalValue(`${field}.focus()`);
		await page.send('Input.insertText', { text: 'r' });
		await clickSelector('header nav a[href="/"]', 'Home, inside the debounce');
		await waitFor(`location.pathname === '/'`, 5000, 'Home followed');
		const settledAt = await evalValue(
			`new Promise((resolve) => setTimeout(() => resolve(location.pathname + location.search), 1000))`
		);
		assert(settledAt === '/', `a link followed while the field waited was pulled back to ${settledAt}`);
		await page.send('Emulation.clearDeviceMetricsOverride');
		console.log('   / focuses, titles suggested, Enter opens results and titles, Escape clears then leaves, a link inside the debounce wins');
	}

	await navigate(`${BASE}/search`);
	await waitText('Search', 10000);
	await fillSearch('frieren');
	await waitFor(
		// A search answers with titles: the two episodes of the fixture's show are one title.
		`document.body.innerText.includes('1 title for')`,
		15000,
		'search results'
	);
	await fillSearch('zzzz-no-such-title');
	await waitText('No matches', 15000);
	await fillSearch('');
	// Leave the field as a person does. Below lg a focused header field
	// opens over the links and they step aside until it is left, so a
	// field still holding the focus would hide Home from the next click.
	await pressKey('Escape', 'Escape', 27);
	await waitFor(`document.activeElement?.id !== 'header-search'`, 5000, 'Escape left the search field');
	console.log('   search results and empty state verified');

	/* ---------------- item + enrichment ---------------- */
	step('item detail and NFO enrichment');
	await clickText('Home');
	await homeRows();
	await clickText('Frieren');
	// Scans auto-enrich now, so strip any existing overlay first: the
	// manual fetch below must exercise the full NFO path. Either button
	// proves the controls rendered (enrichment loads async after the item).
	// The admin controls sit behind the title's Manage menu.
	const menuHas = (text) =>
		evalValue(`[...document.querySelectorAll('[role="menuitem"]')].some((e) => (e.textContent || '').includes(${JSON.stringify(text)}))`);
	await waitText('More', 15000);
	await clickText('More');
	await waitFor(`document.querySelectorAll('[role="menuitem"]').length > 0`, 5000, 'manage menu');
	// The overlay arrives after the item, and the entry turns from "Fetch"
	// into "Refetch" under the pointer when it does: give it a moment to say
	// which one this title has before acting on either.
	let enriched = false;
	for (const until = Date.now() + 3000; Date.now() < until && !enriched; ) {
		enriched = await menuHas('Refetch metadata');
		if (!enriched) await sleep(150);
	}
	if (enriched) {
		await clickText('Remove metadata', '[role="menuitem"]');
		await clickText('Remove overlay');
		await waitText('Metadata overlay removed', 10000);
		await clickText('More');
		await waitFor(`document.querySelectorAll('[role="menuitem"]').length > 0`, 5000, 'manage menu');
	}
	await clickText('Fetch metadata', '[role="menuitem"]');
	await waitText('Metadata added from', 15000);
	// The title hero is set in capitals by the stylesheet, and innerText
	// reports text as styled: the enriched name is looked for in any case.
	await waitFor(
		`document.body.innerText.toLowerCase().includes("frieren: beyond journey's end")`,
		10000,
		'the enriched title is shown'
	);
	console.log('   local NFO overlay applied without network providers');

	/* ---------------- favorites ---------------- */
	// The heart beside a title's main action keeps it on the Favorites page
	// (the header link), and F on the title page does the same: both write
	// one state, so the heart turned on by a click is turned off by the key.
	step('a heart keeps a title on the Favorites page');
	{
		const titlePath = await evalValue('location.pathname');
		const heartOff = 'button[aria-pressed="false"][aria-label="Add to favorites"]';
		const heartOn = 'button[aria-pressed="true"][aria-label="Favorite"]';
		await waitFor(`!!document.querySelector('${heartOff}')`, 15000, 'the heart beside the main action');
		await clickSelector(heartOff, 'the heart');
		await waitFor(`!!document.querySelector('${heartOn}')`, 10000, 'the heart turned on');
		await waitText('Added to your favorites.', 5000);
		await clickText('Favorites', 'header nav a');
		await waitFor(`location.pathname === '/favorites'`, 10000, 'the Favorites page');
		await waitFor(
			`[...document.querySelectorAll('main a[href^="/item/"]')].some((a) => a.textContent.toLowerCase().includes('frieren'))`,
			15000,
			'the kept title is listed'
		);
		await navigate(`${BASE}${titlePath}`);
		await waitFor(`!!document.querySelector('${heartOn}')`, 15000, 'the heart remembers the favourite');
		await page.send('Page.bringToFront');
		await evalValue('document.activeElement?.blur?.()');
		await pressKey('f', 'KeyF', 70);
		await waitFor(`!!document.querySelector('${heartOff}')`, 10000, 'F took the favourite back');
		await navigate(`${BASE}/favorites`);
		await waitText('No favorites yet', 15000);
		console.log('   heart -> listed on /favorites, F -> gone again');
	}

	/* ---------------- title page + watch sidebar ---------------- */
	// The detail route is a *title* page, not an episode page: it lists the
	// show's episodes and every card plays one. The player then keeps that
	// list beside the video so switching episodes never leaves the page.
	step('a show opens a title page with an episode grid');
	await page.send('Emulation.setDeviceMetricsOverride', {
		width: 1440,
		height: 900,
		deviceScaleFactor: 1,
		mobile: false
	});
	await navigate(`${BASE}/library`);
	await clickText('Frieren');
	await waitText('Episodes', 15000);
	const grid = await evalValue(`(() => {
		const links = [...document.querySelectorAll('[data-episode-grid] a[href^="/player/"]')];
		return { count: links.length, hrefs: links.map((a) => a.getAttribute('href')) };
	})()`);
	assert(grid.count >= 2, `episode grid lists ${grid.count} episode(s), want >= 2`);
	const gridHeading = await evalValue(`document.querySelector('h1')?.textContent?.trim() ?? ''`);
	assert(gridHeading.includes('Frieren'), `title page heading: ${JSON.stringify(gridHeading)}`);
	console.log(`   title page: ${grid.count} episode cards, heading ${JSON.stringify(gridHeading)}`);

	step('the player lists the title episodes beside the video');
	await navigate(`${BASE}${grid.hrefs[1]}`);
	await waitFor(`!!document.querySelector('video')`, 15000, 'watch page video');
	const sidebar = await evalValue(`(() => {
		const aside = document.querySelector('aside[aria-label="Episodes"]');
		if (!aside) return null;
		return {
			episodes: aside.querySelectorAll('a[href^="/player/"]').length,
			width: aside.getBoundingClientRect().width,
			current: aside.querySelector('a[aria-current="true"]')?.getAttribute('href') ?? null
		};
	})()`);
	assert(sidebar, 'no episode sidebar on the watch page');
	assert(sidebar.episodes >= 2, `sidebar lists ${sidebar.episodes} episode(s), want >= 2`);
	assert(sidebar.width > 100, `sidebar is ${sidebar.width}px wide at 1440px`);
	assert(sidebar.current === grid.hrefs[1], `current episode marker: ${sidebar.current}`);

	// Clicking another episode changes the route without a page load, so the
	// page has to follow the param: onMount alone would leave the previous
	// episode's state on screen with the marker on the old row.
	const otherRow = await evalValue(`(() => {
		const a = document.querySelector('aside[aria-label="Episodes"] a[href="${grid.hrefs[0]}"]');
		if (!a) return null;
		a.scrollIntoView({ block: 'center' });
		const r = a.getBoundingClientRect();
		return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
	})()`);
	assert(otherRow, `no sidebar row for ${grid.hrefs[0]}`);
	for (const type of ['mousePressed', 'mouseReleased']) {
		await page.send('Input.dispatchMouseEvent', {
			type,
			x: otherRow.x,
			y: otherRow.y,
			button: 'left',
			clickCount: 1
		});
	}
	await waitFor(`location.pathname === '${grid.hrefs[0]}'`, 15000, 'sidebar switched episode');
	await waitFor(
		`document.querySelector('aside[aria-label="Episodes"] a[aria-current="true"]')?.getAttribute('href') === '${grid.hrefs[0]}'`,
		15000,
		'current marker followed the switch'
	);
	await waitFor(`document.querySelector('video').duration > 0`, 20000, 'switched episode loaded');
	await page.send('Emulation.clearDeviceMetricsOverride');
	console.log(`   sidebar switched to ${grid.hrefs[0]} in place, video loaded`);

	step('mkv container plays through the transcode endpoint (HLS fMP4)');
	// The operator's transcode policy, read and written as the admin the
	// way the settings page does; steps that need another policy set it
	// through here and put the shipped value back.
	const adminToken = await evalValue(`localStorage.getItem('lain.token')`);
	assert(adminToken, 'no admin token in localStorage');
	const readSettings = async () => {
		const res = await fetch(`${BASE}/api/admin/settings/transcode`, {
			headers: { Authorization: 'Bearer ' + adminToken }
		});
		assert(res.status === 200, `settings get ${res.status}`);
		return (await res.json()).settings;
	};
	const writeTranscodeSettings = async (patch) => {
		const settings = await readSettings();
		Object.assign(settings, patch);
		const res = await fetch(`${BASE}/api/admin/settings/transcode`, {
			method: 'PUT',
			headers: { Authorization: 'Bearer ' + adminToken, 'Content-Type': 'application/json' },
			body: JSON.stringify(settings)
		});
		assert(res.status === 200, `settings put ${res.status}`);
	};
	const writeDelivery = (delivery) => writeTranscodeSettings({ default_delivery: delivery });
	const sessionState = async (itemId, session) => {
		const res = await fetch(
			`${BASE}/api/items/${itemId}/transcode/status?session=${session}`,
			{ headers: { Authorization: 'Bearer ' + adminToken } }
		);
		assert(res.status === 200, `transcode status ${res.status}`);
		return (await res.json()).state;
	};
	// The replay step below is about a *finished* session. The throttle
	// keeps the encoder only 30s ahead of a viewer, so with it a session
	// finishes no sooner than it is watched; without it the two-minute
	// fixture is done in seconds on any machine. Put back after the replay.
	const shippedThrottle = (await readSettings()).throttle;
	await writeTranscodeSettings({ throttle: false });
	await navigate(`${BASE}/library`);
	await waitText('Other Show');

	// Unenriched cards fall back to a still extracted and cached by the
	// server; the network gate below would also catch a 404 here. A cold
	// server runs ffmpeg once per still, so wait for the response this
	// assertion is about, not merely for the request to leave the browser.
	const thumbDeadline = Date.now() + 15000;
	let thumbResponses = [];
	while (Date.now() < thumbDeadline) {
		thumbResponses = requests.filter(
			(r) => r.url.includes('/thumbnail') && r.status !== null
		);
		if (thumbResponses.length > 0) break;
		await sleep(150);
	}
	assert(thumbResponses.length > 0, 'no thumbnail fallback requests observed');
	assert(
		thumbResponses.every((r) => r.status === 200 || r.status === 304),
		'thumbnail responses: ' + JSON.stringify(thumbResponses.map((r) => r.status))
	);
	console.log(`   ${thumbResponses.length} thumbnail frame(s) served to unenriched cards`);

	await clickText('Other Show');
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance'
	);
	await clickText(
		await evalValue(`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`)
	);
	await waitFor(`!!document.querySelector('video')`, 15000, 'video element');
	await waitFor(`document.querySelector('video').duration > 0`, TRANSCODE_MS, 'transcoded metadata');

	// The MKV container is not web-safe, so the plan is a transcode and the
	// shipped delivery is HLS (D-042): the browser must fetch the
	// server-owned playlist plus fMP4 segments and then actually advance,
	// not merely receive a 200 on the session start.
	await waitUntil(
		() =>
			requests.some(
				(r) => r.url.includes('/transcode') && r.method === 'POST' && r.status === 202
			),
		TRANSCODE_MS,
		'transcode start (202)'
	);
	const hlsPlaylists = () =>
		requests.filter(
			(r) =>
				r.url.includes('/transcode/hls/') && r.url.includes('index.m3u8') && r.status === 200
		);
	await waitUntil(() => hlsPlaylists().length > 0, TRANSCODE_MS, 'HLS playlist');
	const hlsSegments = () =>
		requests.filter(
			(r) => r.url.includes('/transcode/hls/') && r.url.includes('.m4s') && r.status === 200
		);
	await waitUntil(() => hlsSegments().length > 0, TRANSCODE_MS, 'HLS media segment');
	evidence.hlsSegments = hlsSegments().length;
	await evalValue(`document.querySelector('video').play()`);
	await waitFor(`document.querySelector('video').currentTime > 0.5`, TRANSCODE_MS, 'HLS playback advancing');
	console.log(
		`   HLS fMP4: playlist + ${evidence.hlsSegments} segment(s) served, playback advancing`
	);

	// The quality menu rebuilds the session at the chosen rung: the server
	// owns the new identity and playback must survive the swap.
	const sessionFromStatus = () => {
		const r = [...requests].reverse().find((x) => x.url.includes('/transcode/status'));
		if (!r) return null;
		const m = r.url.match(/session=([0-9a-f]+)/);
		return m ? m[1] : null;
	};
	const firstSession = sessionFromStatus();
	assert(firstSession, 'no transcode session observed before the quality change');
	const otherShowId = await evalValue(`location.pathname.split('/').pop()`);
	// Finished before the player is left, so the replay has a cached
	// session to attach to (the player stops an unfinished one on leaving).
	{
		const deadline = Date.now() + TRANSCODE_MS;
		let state = '';
		while (Date.now() < deadline) {
			state = await sessionState(otherShowId, firstSession);
			if (state === 'ready' || state === 'failed') break;
			await sleep(500);
		}
		assert(state === 'ready', `the first HLS session did not finish: ${state}`);
	}

	/* ---------------- cached HLS replay ---------------- */
	step('a cached HLS session replays through hls.js (not progressive)');
	await navigate(`${BASE}/library`);
	// navigate() clears the request log, so count from zero after it.
	const playlistsBeforeReplay = hlsPlaylists().length;
	await clickText('Other Show');
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance (replay)'
	);
	await clickText(
		await evalValue(
			`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`
		)
	);
	await waitFor(`!!document.querySelector('video')`, 15000, 'replay video element');
	// The plan's ready path used to lose the delivery, so the cached HLS
	// session was requested as a progressive MP4 and the gateway answered
	// 409: no playlist, no playback. Assert both halves here.
	await waitUntil(
		() => hlsPlaylists().length > playlistsBeforeReplay,
		TRANSCODE_MS,
		'cached HLS replay playlist'
	);
	await waitFor(
		`document.querySelector('video').duration > 0`,
		TRANSCODE_MS,
		'cached HLS replay metadata'
	);
	const progressive409 = requests.filter(
		(r) => r.url.includes('/transcode?') && r.status === 409
	);
	assert(
		progressive409.length === 0,
		'a cached HLS session was requested as progressive (409): ' + progressive409.length
	);
	// Attached, not prepared again: no session was started for the replay.
	assert(
		!requests.some((r) => r.url.includes('/transcode') && r.method === 'POST'),
		'the replay started a new session instead of attaching the cached one'
	);
	assert(
		sessionFromStatus() === firstSession,
		`the replay attached ${sessionFromStatus()}, not the cached ${firstSession}`
	);
	console.log('   cached HLS replay re-attached through hls.js');

	await openPlayerSettings();
	await waitFor(
		`!!document.querySelector('select[aria-label="Stream quality"]')`,
		15000,
		'quality menu'
	);
	// The ladder stops at the file's own size (640x360, the 360p rung): a
	// larger rung would leave the picture as it is.
	const rungs = await evalValue(
		`[...document.querySelector('select[aria-label="Stream quality"]').options].map((o) => o.value)`
	);
	assert(
		JSON.stringify(rungs) === JSON.stringify(['', '360p']),
		`the quality menu of a 360p file offers ${JSON.stringify(rungs)}, want Auto and 360p`
	);
	await selectOption('Stream quality', '360p');
	await waitUntil(
		() => sessionFromStatus() && sessionFromStatus() !== firstSession,
		TRANSCODE_MS,
		'quality change rebuilt the session'
	);
	await waitFor(
		`document.querySelector('video').currentTime > 0.5`,
		TRANSCODE_MS,
		'playback continued after the quality change'
	);
	evidence.qualitySessionChanged = true;
	console.log(
		`   quality menu rebuilt the session (${firstSession.slice(0, 8)}… -> ${sessionFromStatus().slice(0, 8)}…)`
	);
	await writeTranscodeSettings({ throttle: shippedThrottle });

	/* ---------------- advanced playback settings ---------------- */
	step('advanced playback settings UI');
	await navigate(`${BASE}/settings/transcoding`);
	// Auto shows a summary; Custom reveals every section.
	await waitText('Policy', 20000);
	await evalValue(`[...document.querySelectorAll('[role=radio]')].find((b) => b.textContent.trim() === 'Custom').click()`);
	// Every section the advanced configuration exposes must render.
	for (const section of [
		'Video engine',
		'Probed capabilities',
		'Delivery',
		'Encoding',
		'Per-codec encoding',
		'Quality ladder',
		'Hardware acceleration',
		'HDR & tone mapping',
		'Audio & subtitles',
		'Performance, resources & storage',
		'Active sessions'
	]) {
		await waitText(section, 20000);
	}
	// The redesign keeps a compact navigator and makes large settings pages
	// searchable without removing any control from the underlying contract.
	const settingsSearch = `document.querySelector('input[aria-label="Find a transcoding setting"]')`;
	await evalValue(`(() => { const input = ${settingsSearch}; input.value = 'hardware'; input.dispatchEvent(new Event('input', { bubbles: true })); })()`);
	await waitFor(
		`!!document.getElementById('hardware') && !document.getElementById('delivery')`,
		5000,
		'playback settings search filtered sections'
	);
	await evalValue(`(() => { const input = ${settingsSearch}; input.value = ''; input.dispatchEvent(new Event('input', { bubbles: true })); })()`);
	await waitFor(
		`!!document.getElementById('hardware') && !!document.getElementById('delivery')`,
		5000,
		'playback settings search restored sections'
	);
	await page.send('Emulation.setDeviceMetricsOverride', {
		width: 390,
		height: 844,
		deviceScaleFactor: 1,
		mobile: true
	});
	await settle();
	assert(
		await evalValue(`document.documentElement.scrollWidth <= document.documentElement.clientWidth`),
		'playback settings overflow horizontally at 390px'
	);
	await page.send('Emulation.clearDeviceMetricsOverride');
	// The app-wide Switch bug (a bare `checked` binding) used to paint every
	// toggle ON regardless of state; assert the real binding by flipping one
	// and watching data-state move, then prove it survives a reload.
	const firstSwitch = `document.querySelectorAll('button[role=switch]')[0]`;
	const beforeState = await evalValue(`(${firstSwitch}).getAttribute('data-state')`);
	await evalValue(`(${firstSwitch}).click()`);
	await waitFor(
		`(${firstSwitch}).getAttribute('data-state') !== ${JSON.stringify(beforeState)}`,
		5000,
		'switch toggled'
	);
	const flippedState = await evalValue(`(${firstSwitch}).getAttribute('data-state')`);
	// Settings stage their changes; the bar that appears applies them.
	const saveSettings = async () => {
		await waitFor(
			`[...document.querySelectorAll('button')].some((b) => b.textContent.trim().startsWith('Apply') && !b.disabled)`,
			5000,
			'the staged-changes bar offers Apply'
		);
		await evalValue(
			`[...document.querySelectorAll('button')].find((b) => b.textContent.trim().startsWith('Apply')).click()`
		);
	};
	await saveSettings();
	await waitUntil(
		() =>
			requests.some(
				(r) => r.url.includes('/settings/transcode') && r.method === 'PUT' && r.status === 200
			),
		10000,
		'transcode settings save'
	);
	await navigate(`${BASE}/settings/transcoding`);
	await waitText('Hardware acceleration', 20000);
	const reloadedState = await evalValue(`(${firstSwitch}).getAttribute('data-state')`);
	assert(
		reloadedState === flippedState,
		`switch did not persist: ${beforeState} -> ${flippedState} -> ${reloadedState}`
	);
	// Restore the shipped value so later steps see the default policy.
	await evalValue(`(${firstSwitch}).click()`);
	await saveSettings();
	await waitUntil(
		() =>
			requests.some(
				(r) => r.url.includes('/settings/transcode') && r.method === 'PUT' && r.status === 200
			),
		10000,
		'transcode settings restore'
	);
	console.log(`   advanced settings render, toggle ${beforeState} -> ${flippedState} persists`);

	/* ---------------- progressive delivery ---------------- */
	step('progressive delivery streams a complete MP4 with Range');
	// The shipped default is HLS, covered above. Flip the operator policy to
	// progressive and prove the retained path still streams a faststart MP4
	// with Range in a real browser — the coverage the HLS default replaced.
	await writeDelivery('progressive');
	await navigate(`${BASE}/library`);
	await clickText('Other Show');
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance (progressive)'
	);
	await clickText(
		await evalValue(`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`)
	);
	await waitFor(`!!document.querySelector('video')`, 15000, 'video element');
	await waitFor(`document.querySelector('video').duration > 0`, TRANSCODE_MS, 'progressive metadata');
	await evalValue(`document.querySelector('video').play()`);
	await waitFor(
		`document.querySelector('video').currentTime > 0.5`,
		TRANSCODE_MS,
		'progressive playback advancing'
	);
	const progressiveRange = requests.filter(
		(r) => r.url.includes('/transcode') && r.headers && r.headers.Range && r.status === 206
	);
	assert(progressiveRange.length > 0, 'no Range request observed on the progressive transcode');
	evidence.progressiveRange = progressiveRange.length;
	// Restore the shipped default so later steps see HLS again.
	await writeDelivery('hls');
	console.log(`   progressive MP4 streamed with ${progressiveRange.length} Range request(s)`);

	/* ---------------- the seek bar spans the media ---------------- */
	// An HLS session is an EVENT playlist that lists only the segments
	// ffmpeg has already written, so under MSE the element's own duration is
	// the produced edge — the bar used to stop there. The probed length from
	// the plan is what makes it span the episode, and a seek past the
	// produced edge has to start producing at the target instead of waiting
	// for the encoder to arrive.
	step('the seek bar spans the episode and a far seek rebuilds the session');
	// A lagging transcode is the condition the bug needs, and the only
	// honest way to reproduce it in a test is to make the encoder slow: a
	// tiny fixture is finished before the first progress report, which
	// leaves nothing to lag behind. "Long Show" has its own session so the
	// produced edge here belongs to this step alone.
	const shippedSettings = await readSettings();
	// Measured on this fixture: veryslow with a single thread encodes about
	// 64 fps, so ten minutes of source needs ~110s — comfortably longer than
	// this step, which is what keeps the session partial. In software: a
	// server that picked a hardware encoder on first boot (NVENC on a
	// workstation) ignores the preset and finishes the rest of the source
	// in under a second, and the session is no longer partial.
	await writeTranscodeSettings({
		h264_preset: 'veryslow',
		thread_count: 1,
		hls_segment_seconds: 2,
		throttle_ahead_sec: 5,
		hardware_acceleration: 'none',
		hardware_encode: false
	});
	await navigate(`${BASE}/library`);
	await clickText('Long Show');
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance (seek bar)'
	);
	await clickText(
		await evalValue(`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`)
	);
	const longShowId = await evalValue(`location.pathname.split('/').pop()`);
	await waitFor(`document.querySelector('video')?.duration > 0`, TRANSCODE_MS, 'seek bar video');
	await waitUntil(() => hlsPlaylists().length > 0, TRANSCODE_MS, 'seek bar playlist');

	const timeline = await evalValue(`(() => {
		const video = document.querySelector('video');
		const thumb = document.querySelector('[aria-label="Seek"]');
		if (!video || !thumb) return null;
		return {
			media: video.duration,
			max: Number(thumb.getAttribute('aria-valuemax'))
		};
	})()`);
	assert(timeline, 'no seek bar on the HLS player');
	// The fixture is ten minutes long and the encoder is deliberately slow,
	// so only a fraction of it exists when the viewer seeks: the bar used to
	// stop at that fraction.
	assert(timeline.media > 500, `media duration ${timeline.media}s, want the 600s source`);
	assert(timeline.max > 500, `seek bar max ${timeline.max}s, want the 600s source`);
	evidence.seekBarMax = Math.round(timeline.max);
	console.log(
		`   seek bar spans ${timeline.max.toFixed(1)}s of a ${timeline.media.toFixed(1)}s episode`
	);

	// Drag the thumb to ~70%: past everything produced, so the player has to
	// start a session there rather than clamp to the edge.
	const drag = await evalValue(`(() => {
		const thumb = document.querySelector('[aria-label="Seek"]');
		const root = thumb.parentElement;
		const r = root.getBoundingClientRect();
		const t = thumb.getBoundingClientRect();
		return {
			from: { x: t.left + t.width / 2, y: t.top + t.height / 2 },
			to: { x: r.left + r.width * 0.7, y: r.top + r.height / 2 }
		};
	})()`);
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: drag.from.x, y: drag.from.y });
	await page.send('Input.dispatchMouseEvent', {
		type: 'mousePressed',
		x: drag.from.x,
		y: drag.from.y,
		button: 'left',
		buttons: 1,
		clickCount: 1
	});
	for (let move = 1; move <= 6; move++) {
		await page.send('Input.dispatchMouseEvent', {
			type: 'mouseMoved',
			x: drag.from.x + ((drag.to.x - drag.from.x) * move) / 6,
			y: drag.to.y,
			button: 'left',
			buttons: 1
		});
		await sleep(40);
	}
	await page.send('Input.dispatchMouseEvent', {
		type: 'mouseReleased',
		x: drag.to.x,
		y: drag.to.y,
		button: 'left',
		buttons: 0,
		clickCount: 1
	});

	// The clock starts at the drag's release: that is when the viewer
	// starts waiting, and the rebuild it triggers can fire before this
	// line would otherwise run.
	const seekStartedAt = Date.now();
	// The drag has to move the bar before anything else can be true: a
	// slider that ignores the gesture would make every later assertion
	// meaningless.
	await waitFor(
		`Number(document.querySelector('[aria-label="Seek"]')?.getAttribute('aria-valuenow') ?? 0) > 300`,
		15000,
		'the drag moved the seek bar'
	);

	// A rebuilt session is a server fact: a second POST /transcode whose body
	// carries the target as start_sec. It is also the only way the frames at
	// the target can exist, so it proves the seek rather than the bar's
	// arithmetic.
	await waitUntil(
		() =>
			requests.some(
				(r) =>
					r.url.includes('/transcode') &&
					r.method === 'POST' &&
					/"start_sec":\s*(\d+(\.\d+)?)/.test(r.postData ?? '') &&
					Number((r.postData.match(/"start_sec":\s*(\d+(\.\d+)?)/) ?? [0, 0])[1]) > 300
			),
		TRANSCODE_MS,
		'a session rebuilt at the seek target'
	);
	const rebuild = requests.find(
		(r) => r.method === 'POST' && /"start_sec":\s*(\d+(\.\d+)?)/.test(r.postData ?? '')
	);
	const seekTarget = Number((rebuild.postData.match(/"start_sec":\s*(\d+(\.\d+)?)/) ?? [0, 0])[1]);

	// The bar comes back at the target and keeps moving: frames there only
	// exist because the session was rebuilt for that position.
	await waitFor(
		`Number(document.querySelector('[aria-label="Seek"]')?.getAttribute('aria-valuenow') ?? 0) > ${seekTarget}`,
		TRANSCODE_MS,
		'the seek target reached the bar'
	);
	// The wait a viewer feels is until pictures are moving again, not until
	// the bar moves.
	await waitFor(
		`document.querySelector('video').currentTime > 0.5`,
		TRANSCODE_MS,
		'the rebuilt session plays'
	);
	const playingAt = Date.now();
	evidence.seekLandedAt = Math.round(seekTarget);
	{
		// The wait a seek costs, phase by phase: the rebuild request, the
		// moment the client learns the playlist is playable (its first HLS
		// fetch), the player attaching, and playback actually starting.
		const after = (predicate) =>
			requests.find((r) => r.ts >= seekStartedAt && predicate(r))?.ts;
		const rebuildAt = after(
			(r) => r.method === 'POST' && (r.postData ?? '').includes('start_sec')
		);
		const playlistAt = after((r) => r.url.includes('index.m3u8') && r.status === 200);
		const segmentAt = after((r) => r.url.includes('.m4s') && r.status === 200);
		const phases = {
			request: rebuildAt ? rebuildAt - seekStartedAt : -1,
			playable: playlistAt && rebuildAt ? playlistAt - rebuildAt : -1,
			attach: segmentAt && playlistAt ? segmentAt - playlistAt : -1,
			play: segmentAt ? playingAt - segmentAt : -1,
			total: playingAt - seekStartedAt
		};
		evidence.seekPhases = phases;
		console.log('   seek phases (ms): ' + JSON.stringify(phases));
	}
	console.log(
		`   dragged past the produced edge: rebuilt at ${seekTarget.toFixed(1)}s and played on from there`
	);
	// A second viewer joins the far-seek session the way another tab of
	// the same account would: the player's own request with a viewer id of
	// its own (D-281). Joined while the slow settings hold, since they are
	// part of the session's identity.
	const secondViewer = 'smoke-second-viewer';
	const joinedSession = await (async () => {
		const farSeek = [...requests]
			.reverse()
			.find(
				(r) =>
					r.method === 'POST' &&
					Number(((r.postData ?? '').match(/"start_sec":\s*(\d+(\.\d+)?)/) ?? [0, 0])[1]) > 300
			);
		const body = JSON.parse(farSeek.postData);
		assert(typeof body.viewer === 'string' && body.viewer !== '', 'the player started its session without a viewer id');
		const res = await fetch(`${BASE}/api/items/${longShowId}/transcode`, {
			method: 'POST',
			headers: { Authorization: 'Bearer ' + adminToken, 'Content-Type': 'application/json' },
			body: JSON.stringify({ ...body, viewer: secondViewer })
		});
		assert(res.status === 200 || res.status === 202, `the second viewer's join answered ${res.status}`);
		return (await res.json()).session;
	})();
	await writeTranscodeSettings({
		h264_preset: shippedSettings.h264_preset,
		thread_count: shippedSettings.thread_count,
		hls_segment_seconds: shippedSettings.hls_segment_seconds,
		throttle_ahead_sec: shippedSettings.throttle_ahead_sec,
		hardware_acceleration: shippedSettings.hardware_acceleration,
		hardware_encode: shippedSettings.hardware_encode
	});
	// The deliberately slow session has served its purpose, and it is still
	// being produced. Leaving the player through the app (Back, a client
	// navigation that unmounts it) releases the player's hold on it. The
	// second viewer still holds it, so it keeps running; once that viewer
	// leaves too it has to stop: left running, it would hold one of the
	// server's runners until the idle timeout and queue every later
	// transcode behind it. The player's own releases are what is checked
	// here (the one its rebuild made and the one leaving made); the test
	// only plays the second viewer.
	{
		const last = [...requests].reverse().find((r) => r.url.includes('/transcode/status'));
		const session = last?.url.match(/session=([0-9a-f]+)/)?.[1];
		assert(session, 'no session observed for the far seek');
		assert(joinedSession === session, `the second viewer joined ${joinedSession}, not the far-seek session ${session}`);
		const state = await sessionState(longShowId, session);
		assert(state === 'queued' || state === 'running', `the far-seek session is ${state}, want it still being produced`);
		const statusCode = async (sid) => {
			const res = await fetch(`${BASE}/api/items/${longShowId}/transcode/status?session=${sid}`, {
				headers: { Authorization: 'Bearer ' + adminToken }
			});
			await res.arrayBuffer();
			return res.status;
		};
		const gone = async (sid) => {
			let code = await statusCode(sid);
			for (const until = Date.now() + 15000; code !== 404 && Date.now() < until; ) {
				await sleep(200);
				code = await statusCode(sid);
			}
			return code;
		};
		// The session the far seek replaced was still being produced and
		// only this player held it: the rebuild's release stopped it.
		const replaced = requests.find((r) => r.url.includes('/transcode/status'))?.url.match(/session=([0-9a-f]+)/)?.[1];
		assert(replaced && replaced !== session, 'no session observed before the far seek');
		const replacedCode = await gone(replaced);
		assert(replacedCode === 404, `the rebuild left the session it replaced in place (status ${replacedCode})`);
		console.log('   the rebuild stopped the unfinished session it replaced');
		await evalValue('history.back()');
		await waitFor(`!location.pathname.startsWith('/player/') && !document.querySelector('video')`, 10000, 'Back left the player');
		await waitUntil(
			() =>
				requests.some(
					(r) =>
						r.method === 'DELETE' &&
						r.url.includes(`session=${session}`) &&
						r.url.includes('viewer=') &&
						r.status === 200
				),
			15000,
			"the player's release of its viewer"
		);
		const kept = await sessionState(longShowId, session);
		assert(
			kept === 'queued' || kept === 'running',
			`the player's release stopped a session another viewer holds (state ${kept})`
		);
		console.log('   leaving the player kept the session the second viewer holds');
		const release = await fetch(
			`${BASE}/api/items/${longShowId}/transcode?session=${session}&viewer=${secondViewer}`,
			{ method: 'DELETE', headers: { Authorization: 'Bearer ' + adminToken } }
		);
		const released = await release.json();
		assert(
			release.status === 200 && released.state === 'idle',
			`the last viewer's release answered ${release.status} ${released.state}, want 200 idle`
		);
		const code = await gone(session);
		assert(code === 404, `the last viewer leaving left the unfinished session in place (status ${code})`);
		console.log('   the last viewer leaving stopped the session it was still producing');
	}

	/* ---------------- direct Matroska (D-058) ---------------- */
	// A browser that reports what it can decode gets the file as it is: no
	// session, no encoder, no playlist — just the Range-capable stream
	// endpoint. Before this, Matroska was transcoded on principle, and a
	// seek cost the eight seconds a session restart costs.
	step('a capable browser direct-plays Matroska over Range');
	await navigate(`${BASE}/library`);
	await clickText('Direct Show');
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance (direct)'
	);
	await clickText(
		await evalValue(`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`)
	);
	await waitFor(`!!document.querySelector('video')`, 15000, 'direct video element');
	await waitFor(`document.querySelector('video').duration > 0`, 20000, 'direct metadata');
	await evalValue(`document.querySelector('video').play()`);
	await waitFor(
		`document.querySelector('video').currentTime > 0.5`,
		20000,
		'direct playback advancing'
	);
	// A picture, not merely an advancing clock: the same evidence the
	// player itself demands before it trusts a direct plan.
	await waitFor(`document.querySelector('video').videoWidth > 0`, 20000, 'direct picture');

	const planRequest = requests.find((r) => /\/api\/items\/[^/]+\/playback\?/.test(r.url));
	assert(planRequest, 'no playback plan request observed');
	const planCaps = new URL(planRequest.url).searchParams.get('caps') ?? '';
	for (const token of ['mkv', 'mkv/h264', 'mkv/aac']) {
		assert(
			planCaps.split(',').includes(token),
			`plan request caps=${JSON.stringify(planCaps)} lacks ${token}`
		);
	}
	assert(
		!requests.some((r) => r.url.includes('/transcode') && r.method === 'POST'),
		'a Matroska file this browser can decode still started a transcode session'
	);
	const directStreams = requests.filter((r) => r.url.includes('/stream'));
	assert(directStreams.length > 0, 'no /stream request observed for the direct play');
	assert(
		directStreams.some((r) => r.headers && r.headers.Range),
		'the direct play was not served over Range'
	);
	evidence.directCaps = planCaps;

	// The seek this slice is about: on a direct play it is a Range request
	// into the same file, so the wait is the disk instead of an encoder.
	// Halfway into a ten-minute file is past anything the element holds.
	const directSeekAt = Date.now();
	await evalValue(`document.querySelector('video').currentTime = 300`);
	await waitFor(
		`document.querySelector('video').currentTime > 300 && !document.querySelector('video').seeking`,
		15000,
		'the direct seek reached the target'
	);
	const directSeekMs = Date.now() - directSeekAt;
	const seekRanges = requests.filter(
		(r) => r.ts >= directSeekAt && r.url.includes('/stream') && r.headers && r.headers.Range
	);
	assert(
		directSeekMs < 4000,
		`a direct-play seek took ${directSeekMs}ms; a session restart costs about 8000ms`
	);
	evidence.directSeekMs = directSeekMs;
	evidence.directSeekRanges = seekRanges.length;
	console.log(
		`   direct-play seek: ${directSeekMs}ms (${seekRanges.length} Range request(s) after the seek)`
	);

	/* ---------------- the claim decides the plan ---------------- */
	// The wire contract, against the real server and the real vocabulary:
	// the same file answers differently for a claim the server believes, a
	// claim covering another codec family, a claim of nothing, and no claim
	// at all. The codec pairing is what keeps "H.264 in Matroska" from
	// licensing "HEVC in Matroska".
	step('a capability claim decides the plan and nothing more');
	{
		await navigate(`${BASE}/library`);
		await clickText('Other Show');
		await waitFor(`location.pathname.startsWith('/item/')`, 15000, 'the HEVC item page');
		const token = await evalValue(`localStorage.getItem('lain.token')`);
		const id = await evalValue(`location.pathname.split('/').pop()`);
		const planFor = async (caps) => {
			const query = caps === null ? '' : `&caps=${caps}`;
			const res = await fetch(`${BASE}/api/items/${id}/playback?client=web${query}`, {
				headers: { Authorization: 'Bearer ' + token }
			});
			assert(res.status === 200, `plan request ${res.status}`);
			return await res.json();
		};
		const claimed = await planFor('mkv,mkv/hevc,mkv/aac');
		const otherCodec = await planFor('mkv,mkv/h264,mkv/aac');
		const empty = await planFor('');
		const absent = await planFor(null);
		assert(claimed.mode === 'direct', `claimed plan=${claimed.mode}, want direct`);
		assert(
			otherCodec.mode === 'transcode',
			`a claim for another codec family planned ${otherCodec.mode}, want transcode`
		);
		assert(empty.mode === 'transcode', `empty claim plan=${empty.mode}, want transcode`);
		assert(absent.mode === 'transcode', `absent claim plan=${absent.mode}, want transcode`);
		evidence.capabilityGate = {
			claimed: claimed.mode,
			otherCodec: otherCodec.mode,
			empty: empty.mode,
			absent: absent.mode
		};
		console.log(
			`   claim=${claimed.mode}, other codec=${otherCodec.mode}, empty=${empty.mode}, absent=${absent.mode}`
		);
	}

	/* ---------------- the claim can be wrong ---------------- */
	// A capability list is a claim, and a claim can be wrong: HEVC in
	// Matroska opens, reports no error and paints nothing (measured: 0x0
	// video size, zero frames, an advancing clock). A client that claims it
	// anyway must not leave the viewer on a black rectangle, so the player
	// proves the first decoded frame and falls back to a transcode. The lie
	// is injected at document start, the only place a browser could lie
	// from.
	step('a lying capability claim falls back to a transcode, never a black screen');
	const lie = await page.send('Page.addScriptToEvaluateOnNewDocument', {
		source: `
			HTMLMediaElement.prototype.canPlayType = () => 'probably';
			if (navigator.mediaCapabilities) {
				navigator.mediaCapabilities.decodingInfo = async () => ({
					supported: true, smooth: true, powerEfficient: true
				});
			}
		`
	});
	await navigate(`${BASE}/library`);
	await clickText('Other Show');
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance (lying client)'
	);
	await clickText(
		await evalValue(`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`)
	);
	await waitFor(`!!document.querySelector('video')`, 15000, 'claimed video element');
	const lyingPlan = requests.find((r) => /\/api\/items\/[^/]+\/playback\?/.test(r.url));
	assert(lyingPlan, 'no playback plan request observed for the lying client');
	const lyingCaps = new URL(lyingPlan.url).searchParams.get('caps') ?? '';
	assert(
		lyingCaps.split(',').includes('mkv/hevc'),
		`the lying client did not claim mkv/hevc: ${JSON.stringify(lyingCaps)}`
	);
	// The server believes the claim, so the first delivery is the file
	// itself — and the file decodes to nothing. The request is awaited
	// rather than sampled: the element fetches it once the player mounts.
	await waitUntil(
		() => requests.some((r) => r.url.includes('/stream')),
		15000,
		'the claimed direct play touched the stream endpoint'
	);
	// Then the player notices there is no picture and starts producing a
	// playable version instead of waiting on a black frame forever.
	await waitUntil(
		() => requests.some((r) => r.url.includes('/transcode') && r.method === 'POST'),
		TRANSCODE_MS,
		'the frame check started a transcode'
	);
	await waitFor(
		`document.querySelector('video').videoWidth > 0 && document.querySelector('video').currentTime > 0.5`,
		TRANSCODE_MS,
		'a picture after the fallback'
	);
	await waitText('Preparing this video for your browser', 20000);
	evidence.frameFallback = true;
	console.log('   the claim was disproved by the first frame and the player fell back');
	await page.send('Page.removeScriptToEvaluateOnNewDocument', { identifier: lie.identifier });

	/* ---------------- playback ---------------- */
	step('playback: start, Range, keyboard seek, progress');
	await navigate(`${BASE}/library`);
	await clickText('Frieren');
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance (frieren)'
	);
	await clickText(
		await evalValue(`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`)
	);
	await waitFor(`!!document.querySelector('video')`, 15000, 'video element');
	await waitFor(`document.querySelector('video').duration > 0`, 20000, 'media metadata');
	await evalValue(`document.querySelector('video').play()`);
	await waitFor(`document.querySelector('video').currentTime > 1.5`, 20000, 'playback advancing');

	const streamRequests = requests.filter((r) => r.url.includes('/stream'));
	assert(streamRequests.length > 0, 'no stream request observed');
	const ranged = streamRequests.filter((r) => r.headers && r.headers.Range);
	assert(ranged.length > 0, 'no Range header observed on stream requests');
	evidence.rangeRequests = ranged.length;

	// Keys go to the focused page: earlier steps open other targets (downloads,
	// external players), so this one is brought to the front and nothing in it
	// holds the focus before the player's keys are pressed.
	await page.send('Page.bringToFront');
	await evalValue('document.activeElement?.blur?.()');
	const beforeSeek = await evalValue(`document.querySelector('video').currentTime`);
	await pressKey('ArrowRight', 'ArrowRight', 39);
	await waitFor(`document.querySelector('video').currentTime > ${beforeSeek} + 4`, 10000, 'keyboard seek forward');
	// The key changes the player's state and the element follows on the next
	// update, as the player-controls checks already wait for.
	await pressKey('m', 'KeyM', 77);
	await waitFor(`document.querySelector('video').muted === true`, 3000, 'M mutes');
	await pressKey('m', 'KeyM', 77);
	await waitFor(`document.querySelector('video').muted === false`, 3000, 'M unmutes');
	await pressKey(' ', 'Space', 32);
	await waitFor(`document.querySelector('video').paused === true`, 5000, 'Space paused');
	await pressKey(' ', 'Space', 32);
	await waitFor(`document.querySelector('video').paused === false`, 5000, 'Space resumed');

	// The seek and the pause wrote at once (lifecycle writes); playing on,
	// nothing else forces one, so the next write is the reporter's 10s
	// interval. The browser reports the lifecycle writes a moment late, often
	// after the resume, so "any write after the resume" is met by them and
	// proves nothing: the periodic write is the first one well after it.
	// Its distance from the write before it proves the interval (a reporter
	// writing every second, or on every timeupdate, answers far sooner), and
	// the count at that moment proves the bound.
	const resumedAt = Date.now();
	const progressPuts = () =>
		requests.filter((r) => r.url.includes('/progress') && r.method === 'PUT');
	const isPeriodic = (r) => r.ts > resumedAt + 5000;
	await waitUntil(
		() => progressPuts().some(isPeriodic),
		25000,
		'a periodic progress write while playing'
	);
	const progressWrites = progressPuts();
	{
		const at = progressWrites.findIndex(isPeriodic);
		const previous = progressWrites[at - 1];
		assert(previous, 'no lifecycle progress write (seek, pause) before the periodic one');
		const gap = progressWrites[at].ts - previous.ts;
		// Timestamps are taken as the browser reports each request: some
		// slack under the 10s interval.
		assert(gap >= 7500, `the periodic progress write came ${gap}ms after the previous write, want the 10s interval`);
		evidence.progressIntervalMs = gap;
	}
	assert(
		progressWrites.length <= 4,
		`too many progress writes: ${progressWrites.length} in ~${Math.round((Date.now() - resumedAt) / 1000) + 2}s`
	);
	evidence.progressWrites = progressWrites.length;
	const streamStatuses = requests
		.filter((r) => r.url.includes('/stream') && r.status)
		.map((r) => r.status);
	assert(
		streamStatuses.some((s) => s === 206 || s === 200),
		'stream responses: ' + JSON.stringify(streamStatuses)
	);
	evidence.streamStatuses = [...new Set(streamStatuses)];
	console.log(
		`   ${evidence.rangeRequests} Range requests, ${evidence.progressWrites} bounded progress writes, ${evidence.progressIntervalMs}ms apart`
	);
	// That wait played ten seconds of a thirty-second episode, and the steps
	// below need it still playing (and Continue watching needs it unfinished):
	// back near the start from the keyboard, five seconds a press.
	for (let press = 0; press < 6; press++) {
		const now = await evalValue(`document.querySelector('video').currentTime`);
		if (now < 4) break;
		await pressKey('ArrowLeft', 'ArrowLeft', 37);
		await waitFor(`document.querySelector('video').currentTime < ${now} - 3`, 5000, 'ArrowLeft seeked back');
	}
	await waitFor(`document.querySelector('video').paused === false`, 5000, 'playing on after the rewind');

	/* ---------------- player chrome idle (D-063) ---------------- */
	step('the player chrome fades while playing and wakes on the pointer or a click');
	// The chrome is a fade, never an unmount: the rows keep their place so the
	// picture never reflows and the zones the pointer has to find stay put
	// (D-063 supersedes D-061's no-fade clause).
	const chromeState = `(() => {
		const header = document.querySelector('header');
		const footer = document.querySelector('footer');
		const video = document.querySelector('video');
		return {
			header: header ? getComputedStyle(header).opacity : null,
			footer: footer ? getComputedStyle(footer).opacity : null,
			footerPointer: footer ? getComputedStyle(footer).pointerEvents : null,
			videoHeight: video ? Math.round(video.getBoundingClientRect().height) : 0,
			paused: video ? video.paused : null
		};
	})()`;
	const middle = await evalValue(`(() => {
		const box = document.querySelector('video').getBoundingClientRect();
		return { x: Math.round(box.left + box.width / 2), y: Math.round(box.top + box.height / 2) };
	})()`);
	const footerY = await evalValue(
		`Math.round(document.querySelector('footer').getBoundingClientRect().top + 6)`
	);
	// Wake it first, so the step proves both directions rather than inheriting
	// whatever the previous step left on screen.
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: middle.x, y: footerY });
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '1'`,
		5000,
		'the pointer near the deck woke the chrome'
	);
	const litState = await evalValue(chromeState);

	// Park the pointer on the picture and leave it there: the chrome has to
	// fade on its own while playback continues.
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: middle.x, y: middle.y });
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '0'`,
		8000,
		'chrome faded while playing'
	);
	const darkState = await evalValue(chromeState);
	assert(darkState.header === '0', `the bar stayed on screen: opacity ${darkState.header}`);
	assert(
		darkState.footerPointer === 'none',
		`hidden chrome still swallows clicks: pointer-events ${darkState.footerPointer}`
	);
	// The rows keep their place: the picture must be the same size faded or
	// not, which is the constraint D-061 recorded and D-063 has to honour.
	assert(
		darkState.videoHeight === litState.videoHeight,
		`the picture reflowed while the chrome faded: ${litState.videoHeight}px -> ${darkState.videoHeight}px`
	);
	assert(darkState.paused === false, 'the chrome faded only because playback stopped');
	evidence.chromeVideoHeight = darkState.videoHeight;

	// The chrome floats over the picture now (D-065): the picture owns the whole
	// player region and the deck's top edge sits inside it, which is the
	// geometry the tiled layout could not have.
	const geometry = await evalValue(`(() => {
		const region = document.querySelector('[aria-label^="Player"]').getBoundingClientRect();
		const video = document.querySelector('video').getBoundingClientRect();
		const footer = document.querySelector('footer').getBoundingClientRect();
		return {
			regionTop: Math.round(region.top),
			regionBottom: Math.round(region.bottom),
			videoTop: Math.round(video.top),
			videoBottom: Math.round(video.bottom),
			footerTop: Math.round(footer.top)
		};
	})()`);
	assert(
		Math.abs(geometry.videoTop - geometry.regionTop) <= 1 &&
			Math.abs(geometry.videoBottom - geometry.regionBottom) <= 1,
		`the picture does not own the player region: video ${geometry.videoTop}..${geometry.videoBottom} vs region ${geometry.regionTop}..${geometry.regionBottom}`
	);
	assert(
		geometry.footerTop < geometry.videoBottom,
		`the deck is not over the picture: deck top ${geometry.footerTop}, picture bottom ${geometry.videoBottom}`
	);
	evidence.chromeOverPicturePx = geometry.videoBottom - geometry.footerTop;

	// A click on the picture wakes the chrome and must not pause: pause is the
	// deck button or Space (D-064).
	for (const type of ['mousePressed', 'mouseReleased']) {
		await page.send('Input.dispatchMouseEvent', {
			type,
			x: middle.x,
			y: middle.y,
			button: 'left',
			buttons: type === 'mousePressed' ? 1 : 0,
			clickCount: 1
		});
	}
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '1'`,
		5000,
		'click on the picture woke the chrome'
	);
	assert(
		await evalValue(`document.querySelector('video').paused === false`),
		'clicking the picture paused playback; only the deck button and Space may'
	);

	// A move across the middle of the picture leaves the chrome hidden: the
	// pointer wakes it near the controls, not anywhere.
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '0'`,
		8000,
		'chrome faded again'
	);
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: middle.x, y: middle.y });
	await sleep(3200);
	assert(
		await evalValue(`getComputedStyle(document.querySelector('footer')).opacity === '0'`),
		'a move across the picture kept the chrome on screen'
	);

	// A mouse click leaves focus on the button it hit, and that must not pin the
	// chrome: only keyboard focus does, or the deck would never fade again after
	// the first click on it (the fullscreen button is the one that showed it).
	const deckButton = await evalValue(`(() => {
		const el = document.querySelector('footer button[aria-label="Back 10 seconds"]');
		if (!el) return null;
		const r = el.getBoundingClientRect();
		return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
	})()`);
	assert(deckButton, 'no deck button to click');
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: deckButton.x, y: deckButton.y });
	for (const type of ['mousePressed', 'mouseReleased']) {
		await page.send('Input.dispatchMouseEvent', {
			type,
			x: deckButton.x,
			y: deckButton.y,
			button: 'left',
			buttons: type === 'mousePressed' ? 1 : 0,
			clickCount: 1
		});
	}
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '1'`,
		5000,
		'the deck click woke the chrome'
	);
	assert(
		await evalValue(
			`document.activeElement === document.querySelector('footer button[aria-label="Back 10 seconds"]')`
		),
		'the deck click left focus elsewhere, so this step would not prove anything'
	);
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '0'`,
		8000,
		'chrome faded again after a click on the deck'
	);

	// A scrub pins the chrome — a drag must not have the controls fade under the
	// hand — and letting go has to hand the chrome back to the idle timer. The
	// drag itself is the last pointer event, so nothing else would re-arm it and
	// the deck would stay on screen for the rest of the episode.
	const thumb = await evalValue(`(() => {
		const el = document.querySelector('[aria-label="Seek"]');
		if (!el) return null;
		const r = el.getBoundingClientRect();
		return { x: Math.round(r.left + r.width / 2), y: Math.round(r.top + r.height / 2) };
	})()`);
	assert(thumb, 'no seek thumb to drag');
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: thumb.x, y: thumb.y });
	await page.send('Input.dispatchMouseEvent', {
		type: 'mousePressed',
		x: thumb.x,
		y: thumb.y,
		button: 'left',
		buttons: 1,
		clickCount: 1
	});
	for (let step = 1; step <= 4; step++) {
		await page.send('Input.dispatchMouseEvent', {
			type: 'mouseMoved',
			x: thumb.x + step * 4,
			y: thumb.y,
			button: 'left',
			buttons: 1
		});
		await sleep(40);
	}
	await page.send('Input.dispatchMouseEvent', {
		type: 'mouseReleased',
		x: thumb.x + 16,
		y: thumb.y,
		button: 'left',
		buttons: 0,
		clickCount: 1
	});
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '1'`,
		5000,
		'the drag kept the chrome on screen'
	);
	await waitFor(
		`getComputedStyle(document.querySelector('footer')).opacity === '0'`,
		8000,
		'chrome faded again after a scrub'
	);
	console.log(
		`   chrome faded in place over a ${darkState.videoHeight}px picture, woke on a click and near the deck`
	);

	/* ---------------- direct-play subtitles ---------------- */
	step('familiar player settings stay usable while playback advances');
	await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: middle.x, y: footerY });
	await openPlayerSettings();
	await waitFor(`!!document.querySelector('#player-settings')`, 5000, 'settings panel opened');
	await selectOption('Playback speed', '1.25');
	assert(await evalValue(`document.querySelector('video').playbackRate === 1.25`), 'speed did not apply');
	await sleep(3000);
	assert(await evalValue(`getComputedStyle(document.querySelector('footer')).opacity === '1'`), 'settings faded while open');
	await selectOption('Playback speed', '1');
	await page.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
	await settle();
	assert(await evalValue(`(() => { const r = document.querySelector('#player-settings').getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth && r.top >= 0; })()`), 'settings overflow mobile viewport');
	const mobileShot = await page.send('Page.captureScreenshot', { format: 'png' });
	writeFileSync(SCREENSHOT.replace('.png', '-mobile.png'), Buffer.from(mobileShot.data, 'base64'));
	await page.send('Emulation.setDeviceMetricsOverride', { width: 1440, height: 900, deviceScaleFactor: 1, mobile: false });
	await settle();
	const playerShot = await page.send('Page.captureScreenshot', { format: 'png' });
	writeFileSync(SCREENSHOT.replace('.png', '-player.png'), Buffer.from(playerShot.data, 'base64'));
	await pressKey('Escape', 'Escape', 27);
	assert(await evalValue(`!document.querySelector('#player-settings') && document.activeElement?.getAttribute('aria-label') === 'Playback settings'`), 'Escape did not close settings and restore focus');

	step('player controls and fullscreen regression checks');
	await verifyPlayerControls({ page, evalValue, pressKey, waitFor, assert });

	step('direct-play subtitle extraction serves playable WebVTT');
	await navigate(`${BASE}/library`);
	await clickText('Frieren');
	// A previous step may have left progress, which replaces the Play button
	// with Resume/Start over; pick whichever affordance is present.
	await waitFor(
		`document.body.innerText.includes('Play') || document.body.innerText.includes('Resume from')`,
		15000,
		'play affordance'
	);
	const affordance = await evalValue(
		`document.body.innerText.includes('Resume from') ? 'Resume from' : 'Play'`
	);
	await clickText(affordance);
	await waitFor(`!!document.querySelector('video')`, 15000, 'video element');
	await openPlayerSettings();
	await waitFor(
		`!!document.querySelector('select[aria-label="Subtitle track"]')`,
		15000,
		'subtitle picker'
	);
	const subIndex = await evalValue(`(() => {
		const el = document.querySelector('select[aria-label="Subtitle track"]');
		if (!el) return null;
		const opt = [...el.options].find((o) => o.value !== '');
		return opt ? opt.value : null;
	})()`);
	assert(subIndex, 'the subbed webm offers no subtitle track');
	await selectOption('Subtitle track', subIndex);
	await waitUntil(
		() =>
			requests.some(
				(r) => r.url.includes('/subtitles') && r.url.includes('stream=') && r.status === 200
			),
		15000,
		'subtitle extraction request'
	);
	const trackSrc = await evalValue(`(() => {
		const t = document.querySelector('video track[src*="/subtitles"]');
		return t ? t.getAttribute('src') : null;
	})()`);
	assert(
		trackSrc && trackSrc.includes('stream=' + subIndex),
		'subtitle track src is not the signed extraction URL: ' + trackSrc
	);
	// A 200 is not enough: the sidecar must parse into cues the browser can
	// render, which is the whole point of on-the-fly extraction.
	await waitFor(
		`(() => { const v = document.querySelector('video'); const tt = v && v.textTracks && v.textTracks[0]; return !!(tt && tt.cues && tt.cues.length > 0); })()`,
		15000,
		'subtitle cues loaded'
	);
	evidence.subtitleCues = await evalValue(
		`document.querySelector('video').textTracks[0].cues.length`
	);
	console.log(
		`   ${evidence.subtitleCues} subtitle cue(s) rendered from /subtitles?stream=${subIndex}`
	);

	step('continue watching and reload resume');
	await navigate(`${BASE}/`);
	await waitText('Continue watching', 20000);
	await waitFor(
		`[...document.querySelectorAll('a[href^="/item/"]')].some((a) => a.textContent.includes('Frieren'))`,
		10000,
		'continue watching card'
	);
	const itemHref = await evalValue(
		`document.querySelector('a[href^="/item/"]').getAttribute('href')`
	);
	await navigate(BASE + itemHref);
	await waitFor(`document.body.innerText.includes('Resume from')`, 15000, 'resume label');
	console.log('   progress persisted and reload resumes');

	step('error states');
	await navigate(`${BASE}/player/does-not-exist`);
	await waitText('no longer exists', 10000);
	await navigate(`${BASE}/item/does-not-exist`);
	await waitText('That item is gone', 10000);
	console.log('   missing item and player routes render product error states');

	/* ---------------- admin surfaces ---------------- */
	step('users, plugins swap and backup');
	await navigate(`${BASE}/settings/users`);
	await waitText('Add user');
	await clickText('Add user');
	await waitText('Add a user', 5000);
	await fillLabel('Username', NORMAL_USER);
	await fillLabel('Password', NORMAL_PASS);
	await clickText('Create user');
	await waitText(NORMAL_USER, 10000);

	// The per-user playback limits dialog is part of the requested advanced
	// configuration; prove it opens, binds its switches and persists. Target
	// the row button (selector 'button'), never the 'Playback' nav tab.
	await clickText('Playback', 'button');
	await waitText('Playback limits', 5000);
	const limitsSwitch = `document.querySelectorAll('[role=dialog] button[role=switch]')[0]`;
	const limitsBefore = await evalValue(`(${limitsSwitch}).getAttribute('data-state')`);
	await evalValue(`(${limitsSwitch}).click()`);
	await waitFor(
		`(${limitsSwitch}).getAttribute('data-state') !== ${JSON.stringify(limitsBefore)}`,
		5000,
		'limits switch toggled'
	);
	const limitsFlipped = await evalValue(`(${limitsSwitch}).getAttribute('data-state')`);
	await clickText('Save limits');
	await waitUntil(
		() =>
			requests.some(
				(r) => r.url.includes('/api/users/') && r.method === 'PATCH' && r.status === 200
			),
		10000,
		'playback limits save'
	);
	await clickText('Playback', 'button');
	await waitText('Playback limits', 5000);
	const limitsAfter = await evalValue(`(${limitsSwitch}).getAttribute('data-state')`);
	assert(
		limitsAfter === limitsFlipped,
		`playback limits did not persist: ${limitsBefore} -> ${limitsFlipped} -> ${limitsAfter}`
	);
	// Restore the default so the later non-admin gating step is unaffected.
	await evalValue(`(${limitsSwitch}).click()`);
	await clickText('Save limits');
	await waitUntil(
		() =>
			requests.filter(
				(r) => r.url.includes('/api/users/') && r.method === 'PATCH' && r.status === 200
			).length >= 2,
		10000,
		'playback limits restore'
	);
	console.log(
		`   per-user playback limits toggle ${limitsBefore} -> ${limitsFlipped} persists`
	);

	await navigate(`${BASE}/settings/plugins`);
	await waitText('Capabilities');
	// The first capability whose job an installed provider can do (a
	// component bound before its install offers nothing to replace with).
	await clickText('Replace', 'button:not([disabled])');
	await waitText('Replace providers', 5000);
	const beforeGen = await evalValue(`(() => {
		const dialog = [...document.querySelectorAll('[role=dialog]')][0];
		if (!dialog) return 0;
		const m = dialog.innerText.match(/generation (\\d+)/);
		return m ? Number(m[1]) : 0;
	})()`);
	assert(beforeGen > 0, 'could not read the active generation from the swap dialog');
	await clickText('Apply as generation');
	await waitText('now at generation', 10000);
	await waitGoneText('Apply as generation', 5000);
	const afterGen = await evalValue(`(() => {
		const m = document.body.innerText.match(/now at generation (\\d+)/);
		return m ? Number(m[1]) : 0;
	})()`);
	assert(afterGen > beforeGen, `generation did not advance: ${beforeGen} -> ${afterGen}`);
	evidence.swap = `${beforeGen} -> ${afterGen}`;

	await navigate(`${BASE}/settings/backup`);
	await waitText('Download a backup');
	await clickText('Download a backup');
	await waitUntil(
		() => requests.some((r) => r.url.includes('/api/admin/backup')),
		5000,
		'backup request'
	);
	// A backup is the lain.backup@1 tar (D-238). Chrome writes it under a
	// temporary name and renames it once complete.
	const backups = () => readdirSync(DOWNLOADS).filter((f) => f.endsWith('.tar'));
	await waitUntil(() => backups().length > 0, 15000, 'the backup file in the download directory');
	const downloaded = backups();
	evidence.backupFile = downloaded[0];
	console.log(`   ${downloaded[0]} downloaded`);

	/* ---------------- non-admin gating ---------------- */
	step('logout and non-admin gating');
	await navigate(`${BASE}/settings`);
	await waitText('Sign out');
	await clickText('Sign out');
	await waitText('Sign in', 10000);
	await fillLabel('Username', NORMAL_USER);
	await fillLabel('Password', NORMAL_PASS);
	await clickText('Sign in');
	await waitText('Home', 15000);
	assert(
		!(await evalValue(`document.body.innerText.includes('Plugins')`)),
		'admin nav visible for normal user'
	);
	// The admin left this browser on an admin section, which /settings
	// reopens: the guard has to land a normal user on their own settings
	// rather than bounce between the two (it used to, forever).
	await navigate(`${BASE}/settings/libraries`);
	await waitFor(`location.pathname === '/settings/profile'`, 10000, 'redirected to the account settings');
	await waitFor(
		`document.querySelector('h1')?.textContent?.includes('Profile') && document.querySelector('nav[aria-label="Settings sections"]')?.innerText.includes('YOU')`,
		10000,
		'the account settings rendered'
	);
	assert(
		!(await evalValue(`document.body.innerText.includes('Media libraries')`)),
		'normal user reached library admin page'
	);
	assert(
		!(await evalValue(`/\\bSERVER\\b/.test(document.querySelector('nav[aria-label="Settings sections"]')?.innerText ?? '')`)),
		'normal user is offered the server settings'
	);
	await navigate(`${BASE}/settings`);
	await waitFor(`location.pathname === '/settings/profile'`, 10000, '/settings skips the admin section');
	console.log('   normal user has no admin nav and admin settings redirect to their profile');

	step('logout/login as admin and deep links');
	await navigate(`${BASE}/settings`);
	await waitText('Sign out');
	await clickText('Sign out');
	await waitText('Sign in', 10000);
	await fillLabel('Username', ADMIN_USER);
	await fillLabel('Password', ADMIN_PASS);
	await clickText('Sign in');
	await waitText('Home', 15000);
	await navigate(`${BASE}/settings/plugins`);
	await waitText('Capabilities', 15000);
	await navigate(`${BASE}/library`);
	await waitText('Library', 10000);

	/* ---------------- responsive layouts ---------------- */
	step('responsive layouts');
	{
		const viewports = [
			{ w: 390, h: 844, name: 'mobile' },
			{ w: 768, h: 1024, name: 'tablet' },
			{ w: 1440, h: 900, name: 'desktop' },
			{ w: 2560, h: 1440, name: 'large desktop' }
		];
		for (const vp of viewports) {
			await page.send('Emulation.setDeviceMetricsOverride', {
				width: vp.w,
				height: vp.h,
				deviceScaleFactor: 1,
				mobile: vp.w < 600
			});
			await navigate(`${BASE}/library`);
			await waitText('Library', 10000);
			const overflow = await evalValue(
				'document.documentElement.scrollWidth - window.innerWidth'
			);
			assert(overflow <= 1, `${vp.name} (${vp.w}px): horizontal overflow of ${overflow}px`);
			if (vp.w < 768) {
				const bottomNav = await evalValue(`(() => {
					return [...document.querySelectorAll('nav[aria-label="Primary"]')].some((nav) => {
						const r = nav.getBoundingClientRect();
						return r.height > 0 && r.top > window.innerHeight / 2 && r.bottom <= window.innerHeight + 1;
					});
				})()`);
				assert(bottomNav, `${vp.name}: bottom navigation is not visible`);
			} else {
				const topNav = await evalValue(`(() => {
					const nav = document.querySelector('header nav[aria-label="Primary"]');
					if (!nav) return false;
					const r = nav.getBoundingClientRect();
					return r.width > 100 && r.top < window.innerHeight / 2;
				})()`);
				assert(topNav, `${vp.name}: desktop top navigation is not visible`);
			}
		}
		await page.send('Emulation.clearDeviceMetricsOverride');
		console.log('   390 / 768 / 1440 / 2560 px: no overflow, correct navigation');
	}

	/* ---------------- quality gates ---------------- */
	step('console and network quality gate');
	{
		// The error-state step deliberately requests a missing item; a
		// 404 there is the product contract, tracked separately below.
		const intentional404 = /\/api\/(catalog|items)\/does-not-exist/;
		const unexpected404 = [...seen404].filter((u) => !intentional404.test(u));
		const appOriginErrors = consoleErrors.filter(
			(e) => !e.includes('ERR_BLOCKED_BY_CLIENT') && !e.includes('status of 404')
		);
		const pageErrors = consoleErrors.filter((e) => e.startsWith('exception:'));
		assert(pageErrors.length === 0, 'uncaught page exceptions:\n' + pageErrors.join('\n'));
		assert(appOriginErrors.length === 0, 'console errors:\n' + appOriginErrors.join('\n'));
		assert(unexpected404.length === 0, 'unexpected 404s:\n' + unexpected404.join('\n'));
	}
	{
		const loops = [...requestCount.entries()].filter(
			([key, n]) => n > 6 && !key.endsWith('/api/theme')
		);
		assert(
			loops.length === 0,
			'possible request loops:\n' + loops.map(([k, n]) => `${n}x ${k}`).join('\n')
		);
		assert(requests.length < 400, `request count too high for this flow: ${requests.length}`);
	}
	console.log('   no console errors, no 404s, no request loops');

	step('report');
	console.log(JSON.stringify(evidence, null, 2));
	console.log('\nSMOKE PASSED');
} catch (err) {
	console.error('\nSMOKE FAILURE:', err?.stack ?? err);
	if (consoleErrors.length) {
		console.error('console/network errors:\n' + consoleErrors.join('\n'));
	}
	if (seen404.size) {
		console.error('404s:\n' + [...seen404].join('\n'));
	}
	if (allConsole.length) {
		console.error('page console:\n' + allConsole.slice(-25).join('\n'));
	}
	console.error(
		'last requests:\n' +
			requests
				.filter((r) => !r.url.endsWith('/api/theme'))
				.slice(-30)
				.map(
					(r) =>
						`+${((r.ts - startedAt) / 1000).toFixed(1)}s ${r.method} ${r.url.replace(BASE, '').replace(/token=[^&]+/, 'token=…')} -> ${r.status}`
				)
				.join('\n')
	);
	for (const pl of playlists) {
		console.error(`playlist at +${((pl.at - startedAt) / 1000).toFixed(1)}s ${pl.url}:\n${pl.text.replace(/token=[^&"\s]+/g, 'token=…')}`);
	}
	try {
		// The player's element as it stood: a stall reads differently from
		// an error, and the buffered ranges say whether data ever arrived.
		const media = await evalValue(`(() => {
			const v = document.querySelector('video');
			if (!v) return null;
			const ranges = [];
			for (let i = 0; i < v.buffered.length; i++) ranges.push([v.buffered.start(i), v.buffered.end(i)]);
			return {
				path: location.pathname,
				src: v.currentSrc.replace(/token=[^&]+/, 'token=…').slice(0, 160),
				currentTime: v.currentTime,
				duration: v.duration,
				readyState: v.readyState,
				networkState: v.networkState,
				paused: v.paused,
				seeking: v.seeking,
				error: v.error ? v.error.code + ' ' + v.error.message : null,
				videoWidth: v.videoWidth,
				buffered: ranges,
				seekNow: document.querySelector('[aria-label="Seek"]')?.getAttribute('aria-valuenow') ?? null
			};
		})()`);
		console.error('video element: ' + JSON.stringify(media));
	} catch {
		/* page gone or not a player */
	}
	try {
		const shot = await page.send('Page.captureScreenshot', { format: 'png' });
		writeFileSync(SCREENSHOT, Buffer.from(shot.data, 'base64'));
		console.error('screenshot: ' + SCREENSHOT);
	} catch {
		/* page already gone */
	}
	exitCode = 1;
} finally {
	try {
		page?.close();
		browser?.close();
	} catch {
		/* already closed */
	}
	chrome.kill('SIGKILL');
}

process.exit(exitCode);
