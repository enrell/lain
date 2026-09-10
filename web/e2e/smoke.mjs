#!/usr/bin/env node
/*
 * Lain browser smoke test.
 *
 * Drives a real Chromium over the DevTools Protocol using only Node
 * built-ins (global fetch + WebSocket) against a *fresh* Lain data
 * directory. It exercises the actual Go server with a real database
 * and real media files:
 *
 *   setup -> libraries -> scan -> browse -> search -> item enrichment
 *   -> playback -> Range -> keyboard seek -> bounded progress writes
 *   -> continue watching -> admin surfaces -> swap fencing
 *   -> backup -> non-admin gating -> deep links -> quality gates
 *
 * Usage:
 *   web/e2e/fixtures.sh /tmp/lain-fixtures          # synthetic media
 *   lain serve --data-dir /tmp/lain-e2e --port 9360 # fresh dir
 *   node web/e2e/smoke.mjs \
 *     --base http://127.0.0.1:9360 \
 *     --media /tmp/lain-fixtures \
 *     --chromium chromium
 *
 * Exits non-zero when any assertion fails or the console/network
 * quality gate trips. Requires Node >= 22 (built-in WebSocket).
 */
import { spawn } from 'node:child_process';
import { mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

const args = process.argv.slice(2);
const arg = (name, fallback) => {
	const i = args.indexOf(`--${name}`);
	return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};

const BASE = arg('base', 'http://127.0.0.1:9360');
const MEDIA = arg('media', '/tmp/lain-e2e-fixtures');
const CHROMIUM = arg('chromium', 'chromium');
const PORT = Number(arg('debug-port', '9333'));
const PROFILE = arg('profile', '/tmp/lain-e2e-chrome');
const DOWNLOADS = arg('downloads', '/tmp/lain-e2e-downloads');
const SCREENSHOT = arg('screenshot', '/tmp/lain-e2e-failure.png');
const ADMIN_USER = arg('user', 'admin');
const ADMIN_PASS = arg('pass', 'password123');
const NORMAL_USER = arg('user2', 'ana');
const NORMAL_PASS = arg('pass2', 'password123');

function assert(cond, message) {
	if (!cond) throw new Error(message);
}

function step(name) {
	process.stdout.write(`\n== ${name}\n`);
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

	send(method, params = {}) {
		const id = ++this.id;
		return new Promise((resolve, reject) => {
			this.pending.set(id, { resolve, reject });
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
	await evalValue(
		'new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve(true))))'
	);
}

async function waitText(text, timeoutMs = 15000) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		const found = await evalValue(
			`document.body ? document.body.innerText.includes(${JSON.stringify(text)}) : false`
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
async function clickText(text, selector = 'button, a') {
	await settle();
	const deadline = Date.now() + 5000;
	let info = null;
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
			const hitEl = hit ? hit.closest('a, button') : null;
			return {
				tag: el.tagName,
				href: el.getAttribute('href'),
				x,
				y,
				hit: hitEl ? (hitEl.textContent || '').trim().slice(0, 60) : (hit ? hit.tagName : null)
			};
		})()`);
		if (info && info.hit && info.hit.includes(text)) break;
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

async function fillPlaceholder(placeholder, value) {
	const filled = await evalValue(`(() => {
		const el = document.querySelector('input[placeholder=' + ${JSON.stringify(JSON.stringify(placeholder))} + ']');
		if (!el) return false;
		const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
		setter.call(el, ${JSON.stringify(value)});
		el.dispatchEvent(new Event('input', { bubbles: true }));
		el.dispatchEvent(new Event('change', { bubbles: true }));
		return true;
	})()`);
	assert(filled, `no input with placeholder ${JSON.stringify(placeholder)}`);
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
	console.log(`browser: ${version['Browser']}`);

	const target = await (
		await fetch(`http://127.0.0.1:${PORT}/json/new?${encodeURIComponent(BASE + '/')}`, {
			method: 'PUT'
		})
	).json();
	page = await CDP.connect(target.webSocketDebuggerUrl);
	browser = await CDP.connect(version.webSocketDebuggerUrl);

	await page.send('Page.enable');
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
	await fillLabel('Username', ADMIN_USER);
	await fillLabel('Password', ADMIN_PASS);
	await fillLabel('Confirm password', ADMIN_PASS);
	await clickText('Create account');
	await waitText("You're in.");
	console.log('   setup account created and authenticated');

	step('create and scan libraries');
	await clickText('Add a media library');
	await waitFor(`location.pathname === '/settings/libraries'`, 10000, 'settings/libraries URL');
	await waitText('Media libraries', 10000);
	await clickText('Add your first library');
	await waitText('Add a media library', 5000);
	await fillLabel('Name', 'Anime');
	await fillLabel('Server directory path', `${MEDIA}/Anime`);
	await clickText('Add and scan');
	await waitFor(`document.body.innerText.includes('identified')`, 20000, 'first scan finished');

	await clickText('Add library');
	await waitText('Add a media library', 5000);
	await fillLabel('Name', 'Movies');
	await fillLabel('Server directory path', `${MEDIA}/Movies`);
	await clickText('Add and scan');
	await waitFor(`document.body.innerText.includes('identified')`, 20000, 'second scan finished');
	console.log('   two libraries scanned');

	/* ---------------- browse ---------------- */
	step('home, catalog and search');
	await clickText('Home');
	await waitText('Recently added');
	await waitFor(
		`[...document.querySelectorAll('a[href^="/item/"]')].length >= 3`,
		15000,
		'recently added grid'
	);
	const cards = await evalValue(`document.querySelectorAll('a[href^="/item/"]').length`);
	console.log(`   ${cards} media cards on Home`);

	await clickText('Search');
	await waitText('Search your library');
	await fillPlaceholder('Titles, not filenames', 'frieren');
	await waitFor(
		`document.body.innerText.includes('2 results') || document.body.innerText.includes('1 result')`,
		15000,
		'search results'
	);
	await fillPlaceholder('Titles, not filenames', 'zzzz-no-such-title');
	await waitText('No matches', 15000);
	await fillPlaceholder('Titles, not filenames', '');
	console.log('   search results and empty state verified');

	/* ---------------- item + enrichment ---------------- */
	step('item detail and NFO enrichment');
	await clickText('Home');
	await waitText('Recently added');
	await clickText('Frieren');
	await waitText('Fetch metadata');
	await clickText('Fetch metadata');
	await waitText('Metadata added from', 15000);
	await waitText("Frieren: Beyond Journey's End", 10000);
	console.log('   local NFO overlay applied without network providers');

	step('unplayable container degrades honestly');
	await navigate(`${BASE}/library`);
	await waitText('Other Show');
	await clickText('Other Show');
	await waitText('cannot play this file directly', 15000);
	const disabledPlay = await evalValue(
		`[...document.querySelectorAll('button')].some((b) => b.disabled && b.textContent.includes('Play'))`
	);
	assert(disabledPlay, 'Play button should be disabled for an unplayable container');
	console.log('   mkv reports transcode-required instead of faking playback');

	/* ---------------- playback ---------------- */
	step('playback: start, Range, keyboard seek, progress');
	await navigate(`${BASE}/library`);
	await clickText('Frieren');
	await waitText('Play', 15000);
	await clickText('Play');
	await waitFor(`!!document.querySelector('video')`, 15000, 'video element');
	await waitFor(`document.querySelector('video').duration > 0`, 20000, 'media metadata');
	await evalValue(`document.querySelector('video').play()`);
	await waitFor(`document.querySelector('video').currentTime > 1.5`, 20000, 'playback advancing');

	const streamRequests = requests.filter((r) => r.url.includes('/stream'));
	assert(streamRequests.length > 0, 'no stream request observed');
	const ranged = streamRequests.filter((r) => r.headers && r.headers.Range);
	assert(ranged.length > 0, 'no Range header observed on stream requests');
	evidence.rangeRequests = ranged.length;

	await pressKey('ArrowRight', 'ArrowRight', 39);
	await waitFor(`document.querySelector('video').currentTime > 6`, 10000, 'keyboard seek forward');
	await pressKey('m', 'KeyM', 77);
	assert(await evalValue(`document.querySelector('video').muted === true`), 'M did not mute');
	await pressKey('m', 'KeyM', 77);
	assert(await evalValue(`document.querySelector('video').muted === false`), 'M did not unmute');
	await pressKey(' ', 'Space', 32);
	await waitFor(`document.querySelector('video').paused === true`, 5000, 'Space paused');
	await pressKey(' ', 'Space', 32);
	await waitFor(`document.querySelector('video').paused === false`, 5000, 'Space resumed');

	// Long enough for the 10s progress interval to fire at least once.
	await sleep(12000);
	const progressWrites = requests.filter(
		(r) => r.url.includes('/progress') && r.method === 'PUT'
	);
	assert(progressWrites.length >= 1, 'no progress write observed after 12s');
	assert(
		progressWrites.length <= 4,
		`too many progress writes: ${progressWrites.length} in ~14s`
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
		`   ${evidence.rangeRequests} Range requests, ${evidence.progressWrites} bounded progress writes`
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

	await navigate(`${BASE}/settings/plugins`);
	await waitText('Capabilities');
	await clickText('Replace');
	await waitText('Replace providers', 5000);
	const beforeGen = await evalValue(`(() => {
		const dialog = [...document.querySelectorAll('[role=dialog]')][0];
		if (!dialog) return 0;
		const m = dialog.innerText.match(/generation (\\d+)/);
		return m ? Number(m[1]) : 0;
	})()`);
	assert(beforeGen > 0, 'could not read the active generation from the swap dialog');
	await clickText('Apply generation');
	await waitText('now generation', 10000);
	await waitGoneText('Apply generation', 5000);
	const afterGen = await evalValue(`(() => {
		const m = document.body.innerText.match(/now generation (\\d+)/);
		return m ? Number(m[1]) : 0;
	})()`);
	assert(afterGen > beforeGen, `generation did not advance: ${beforeGen} -> ${afterGen}`);
	evidence.swap = `${beforeGen} -> ${afterGen}`;

	await navigate(`${BASE}/settings/backup`);
	await waitText('Download lain.db');
	await clickText('Download lain.db');
	await waitUntil(
		() => requests.some((r) => r.url.includes('/api/admin/backup')),
		5000,
		'backup request'
	);
	await sleep(1500);
	const downloaded = readdirSync(DOWNLOADS).filter((f) => f.endsWith('.db'));
	assert(downloaded.length > 0, 'no backup file reached the download directory');
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
	await navigate(`${BASE}/settings/libraries`);
	await waitText('Your account.', 10000);
	assert(
		!(await evalValue(`document.body.innerText.includes('Media libraries')`)),
		'normal user reached library admin page'
	);
	console.log('   normal user has no admin nav and /settings/libraries redirects');

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

	/* ---------------- quality gates ---------------- */
	step('console and network quality gate');
	{
		const appOriginErrors = consoleErrors.filter((e) => !e.includes('ERR_BLOCKED_BY_CLIENT'));
		const pageErrors = consoleErrors.filter((e) => e.startsWith('exception:'));
		assert(pageErrors.length === 0, 'uncaught page exceptions:\n' + pageErrors.join('\n'));
		assert(appOriginErrors.length === 0, 'console errors:\n' + appOriginErrors.join('\n'));
		assert(seen404.size === 0, 'unexpected 404s:\n' + [...seen404].join('\n'));
	}
	{
		const loops = [...requestCount.entries()].filter(([, n]) => n > 6);
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
	console.error(
		'last requests:\n' +
			requests
				.slice(-20)
				.map((r) => `${r.method} ${r.url.replace(BASE, '')} -> ${r.status}`)
				.join('\n')
	);
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
