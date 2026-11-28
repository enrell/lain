#!/usr/bin/env node
/*
 * Lain books browser test (D-303..D-308): a fresh server with a library
 * of type `book` holding a tiny EPUB and a tiny PDF (the conformance
 * fixtures), driven in a headless Chromium over the DevTools Protocol:
 *
 *   scan -> the book's manifest -> the EPUB reader (sanitized chapter in
 *   a frame that runs no script, a page turn into the next chapter by
 *   the keyboard from inside the frame, the contents dialog, the text
 *   settings, progress saved in virtual pages) -> the PDF reader (a
 *   page rendered by the server at the width shown) -> the card cover.
 *
 *   node web/e2e/books.mjs --base http://127.0.0.1:19490 --media DIR [--shots DIR]
 *
 * DIR must hold the fixture books; the library points at it. Exits
 * non-zero on the first failed assertion, with a screenshot.
 */
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const args = process.argv.slice(2);
const arg = (name, fallback) => {
	const i = args.indexOf(`--${name}`);
	return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};
const BASE = arg('base', 'http://127.0.0.1:19490');
const MEDIA = arg('media', '');
const SHOTS = arg('shots', '/tmp/lain-books-e2e');
const PORT = Number(arg('debug-port', '19491'));
const PROFILE = join(SHOTS, 'chrome');

function findChrome() {
	for (const dir of (process.env.PATH ?? '').split(':').filter(Boolean)) {
		for (const name of ['chromium', 'chromium-browser', 'google-chrome-stable', 'google-chrome']) {
			if (existsSync(join(dir, name))) return join(dir, name);
		}
	}
	return 'chromium';
}

function assert(cond, message) {
	if (!cond) throw new Error(message);
}

function step(name) {
	process.stdout.write(`== ${name}\n`);
}

async function api(method, path, body, token) {
	const res = await fetch(BASE + path, {
		method,
		headers: { ...(body ? { 'content-type': 'application/json' } : {}), ...(token ? { authorization: `Bearer ${token}` } : {}) },
		body: body ? JSON.stringify(body) : undefined
	});
	const text = await res.text();
	let json = null;
	try {
		json = JSON.parse(text);
	} catch {
		json = text;
	}
	return { status: res.status, json, headers: res.headers };
}

mkdirSync(SHOTS, { recursive: true });
rmSync(PROFILE, { recursive: true, force: true });
const chrome = spawn(
	process.env.CHROME || findChrome(),
	[
		'--headless=new',
		'--disable-gpu',
		'--no-first-run',
		'--no-default-browser-check',
		'--disable-dev-shm-usage',
		'--disable-extensions',
		'--disable-background-timer-throttling',
		'--disable-renderer-backgrounding',
		'--window-size=1280,860',
		...(process.env.CHROME_FLAGS ?? '').split(/\s+/).filter(Boolean),
		`--remote-debugging-port=${PORT}`,
		`--user-data-dir=${PROFILE}`,
		'about:blank'
	],
	{ stdio: 'ignore' }
);

class CDP {
	constructor(ws) {
		this.ws = ws;
		this.id = 0;
		this.pending = new Map();
		this.listeners = new Map();
		ws.addEventListener('message', (event) => {
			const msg = JSON.parse(event.data);
			if (msg.id !== undefined) {
				const p = this.pending.get(msg.id);
				if (!p) return;
				this.pending.delete(msg.id);
				if (msg.error) p.reject(new Error(msg.error.message));
				else p.resolve(msg.result);
				return;
			}
			for (const fn of this.listeners.get(msg.method) ?? []) fn(msg.params);
		});
	}
	static async connect(url) {
		const ws = new WebSocket(url);
		await new Promise((resolve, reject) => {
			ws.addEventListener('open', resolve, { once: true });
			ws.addEventListener('error', () => reject(new Error('websocket failed')), { once: true });
		});
		return new CDP(ws);
	}
	send(method, params = {}) {
		const id = ++this.id;
		return new Promise((resolve, reject) => {
			const timer = setTimeout(() => reject(new Error(`CDP ${method} timed out`)), 30000);
			this.pending.set(id, {
				resolve: (v) => (clearTimeout(timer), resolve(v)),
				reject: (e) => (clearTimeout(timer), reject(e))
			});
			this.ws.send(JSON.stringify({ id, method, params }));
		});
	}
	on(method, fn) {
		if (!this.listeners.has(method)) this.listeners.set(method, []);
		this.listeners.get(method).push(fn);
	}
}

let page;
const consoleErrors = [];
const progressWrites = [];

async function evalValue(expression) {
	const { result, exceptionDetails } = await page.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
	if (exceptionDetails) throw new Error('page exception: ' + (exceptionDetails.exception?.description ?? exceptionDetails.text));
	return result.value;
}

async function waitFor(expression, timeoutMs = 15000, label = expression) {
	const deadline = Date.now() + timeoutMs;
	while (Date.now() < deadline) {
		try {
			if (await evalValue(`!!(${expression})`)) return;
		} catch {
			/* a navigation in flight */
		}
		await sleep(120);
	}
	throw new Error('timed out waiting for ' + label);
}

async function key(k, code, keyCode, shift = false) {
	// The reader ignores page turns closer than 110 ms (a held key): a
	// fast machine would otherwise lose a turn right after a chapter loads.
	await sleep(200);
	for (const type of ['keyDown', 'keyUp']) {
		await page.send('Input.dispatchKeyEvent', { type, key: k, code, windowsVirtualKeyCode: keyCode, modifiers: shift ? 8 : 0 });
	}
}

async function shot(name) {
	const { data } = await page.send('Page.captureScreenshot', { format: 'png' });
	writeFileSync(join(SHOTS, `${name}.png`), Buffer.from(data, 'base64'));
}

const frameText = `(document.querySelector('[data-book-reader][data-loading=false] iframe')?.contentDocument?.body?.innerText ?? '')`;

let code = 0;
try {
	assert(MEDIA, '--media is required');
	step('setup, a book library and a scan');
	assert((await api('POST', '/api/setup', { username: 'admin', password: 'password123' })).status === 201, 'setup');
	const token = (await api('POST', '/api/auth/login', { username: 'admin', password: 'password123' })).json.token;
	const lib = await api('POST', '/api/libraries', { name: 'Books', type: 'book', path: MEDIA }, token);
	assert(lib.status === 201, 'library: ' + JSON.stringify(lib.json));
	await api('POST', '/api/library/scan', null, token);
	let items = [];
	for (let i = 0; i < 100 && items.length < 2; i++) {
		await sleep(200);
		const res = await api('GET', `/api/catalog?library_id=${lib.json.id}`, null, token);
		items = (Array.isArray(res.json) ? res.json : (res.json?.items ?? [])).filter((it) => it.kind === 'book');
	}
	assert(items.length >= 2, 'two books scanned: ' + JSON.stringify(items));
	const epub = items.find((it) => it.file_path.endsWith('.epub') && !it.file_path.includes('broken'));
	const pdf = items.find((it) => it.file_path.endsWith('.pdf'));
	assert(epub && pdf, 'an epub and a pdf');

	step('the manifests');
	const m = await api('GET', `/api/items/${epub.id}/book`, null, token);
	assert(m.status === 200 && m.json.title === 'Tiny Book' && m.json.chapters.length === 2, 'epub manifest: ' + JSON.stringify(m.json));
	const pm = await api('GET', `/api/items/${pdf.id}/book`, null, token);
	assert(pm.status === 200 && pm.json.layout === 'pages' && pm.json.pages === 2, 'pdf manifest');

	step('the browser');
	const deadline = Date.now() + 15000;
	let target;
	while (!target && Date.now() < deadline) {
		try {
			const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
			target = list.find((t) => t.type === 'page');
		} catch {
			await sleep(150);
		}
	}
	assert(target, 'chromium did not start');
	page = await CDP.connect(target.webSocketDebuggerUrl);
	await page.send('Runtime.enable');
	await page.send('Network.enable');
	page.on('Runtime.exceptionThrown', (p) => consoleErrors.push(p.exceptionDetails?.exception?.description ?? p.exceptionDetails?.text));
	page.on('Runtime.consoleAPICalled', (p) => {
		if (p.type === 'error') consoleErrors.push(p.args.map((a) => a.value ?? a.description).join(' '));
	});
	page.on('Network.requestWillBeSent', (p) => {
		if (p.request.method === 'PUT' && p.request.url.includes('/progress')) progressWrites.push(p.request.postData ?? '');
	});
	await page.send('Page.navigate', { url: BASE + '/login' });
	await waitFor(`document.readyState === 'complete'`);
	await evalValue(`localStorage.setItem('lain.token', ${JSON.stringify(token)}), true`);

	step('the EPUB reader');
	await page.send('Page.navigate', { url: `${BASE}/read/${epub.id}` });
	await waitFor(`${frameText}.includes('The first page of a tiny book')`, 20000, 'the first chapter in the frame');
	const leaked = await evalValue(`(() => { const d = document.querySelector('[data-book-reader] iframe').contentDocument; return d.querySelectorAll('script, [onclick]').length; })()`);
	assert(leaked === 0, 'no script or handler reached the frame');
	const imgOk = await evalValue(`new Promise((r) => { const d = document.querySelector('[data-book-reader] iframe').contentDocument; const i = d.querySelector('img'); if (!i) return r(false); const ok = () => r(i.naturalWidth > 0); i.complete ? ok() : (i.onload = ok, i.onerror = () => r(false)); })`);
	assert(imgOk, 'the chapter picture loaded through the resource route with the token');
	await shot('epub-chapter-1');

	// Keys typed inside the frame reach the reader.
	await evalValue(`document.querySelector('[data-book-reader] iframe').contentWindow.focus(), true`);
	await key('ArrowRight', 'ArrowRight', 39);
	await waitFor(`${frameText}.includes('The end.')`, 10000, 'the next chapter after a page turn');
	await key('ArrowLeft', 'ArrowLeft', 37);
	await waitFor(`${frameText}.includes('The first page')`, 10000, 'back to the first chapter');

	step('links between chapters');
	await evalValue(`document.querySelector('[data-book-reader] iframe').contentDocument.querySelector('a[data-lain-chapter]').click(), true`);
	await waitFor(`${frameText}.includes('The end.')`, 10000, 'the link opened chapter two');

	step('contents and text settings');
	await key('t', 'KeyT', 84);
	await waitFor(`[...document.querySelectorAll('[role=dialog] button')].some((b) => b.textContent.trim() === 'One')`, 5000, 'the contents dialog');
	await evalValue(`[...document.querySelectorAll('[role=dialog] button')].find((b) => b.textContent.trim() === 'One').click(), true`);
	await waitFor(`${frameText}.includes('The first page')`, 10000, 'the contents entry opened chapter one');
	await key('=', 'Equal', 187);
	await waitFor(`document.querySelector('[data-book-reader] iframe').contentDocument.getElementById('lain-reader').textContent.includes('font-size: 110%')`, 5000, 'a bigger text size');
	await key('a', 'KeyA', 65);
	await waitFor(`!!document.querySelector('[role=dialog]') && document.querySelector('[role=dialog]').innerText.includes('Line spacing')`, 5000, 'the text settings dialog');
	await shot('epub-settings');
	await key('Escape', 'Escape', 27);
	await waitFor(`!document.querySelector('[role=dialog]')`, 5000, 'the dialog closed');

	step('progress is saved in virtual pages');
	await key('End', 'End', 35);
	await key('ArrowRight', 'ArrowRight', 39);
	await waitFor(`${frameText}.includes('The end.')`, 10000, 'chapter two again');
	await key('ArrowRight', 'ArrowRight', 39);
	await waitFor(`!!document.querySelector('[role=dialog][aria-label="The end"]')`, 5000, 'the end card');
	await sleep(2600);
	const saved = await api('GET', `/api/items/${epub.id}/progress`, null, token);
	assert(saved.status === 200 && saved.json.duration_sec === m.json.pages && saved.json.completed === true, 'finished progress: ' + JSON.stringify(saved.json));
	await shot('epub-end');

	step('the title page and reading offline');
	await page.send('Page.navigate', { url: `${BASE}/item/${epub.id}` });
	await waitFor(`document.body.innerText.includes('A. Writer') && document.body.innerText.includes('EPUB')`, 15000, 'the book panel');
	await key('o', 'KeyO', 79);
	await waitFor(`[...document.querySelectorAll('button')].some((b) => /Remove offline|Remove from offline|offline copy/i.test(b.textContent || '') && !/Saving/.test(b.textContent || ''))`, 20000, 'the saved state');
	await page.send('Network.emulateNetworkConditions', { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
	await page.send('Page.navigate', { url: `${BASE}/read/${epub.id}` });
	await waitFor(`${frameText}.includes('The first page')`, 20000, 'the chapter read offline');
	const offlineImg = await evalValue(`new Promise((r) => { const i = document.querySelector('[data-book-reader] iframe').contentDocument.querySelector('img'); if (!i) return r(false); const ok = () => r(i.naturalWidth > 0); i.complete ? ok() : (i.onload = ok, i.onerror = () => r(false)); })`);
	assert(offlineImg, 'the chapter picture came from the offline copy');
	await shot('epub-offline');
	await page.send('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
	consoleErrors.length = 0;

	step('the PDF reader');
	await page.send('Page.navigate', { url: `${BASE}/read/${pdf.id}` });
	await waitFor(`[...document.images].some((i) => i.src.includes('/pages/0?') && i.src.includes('w=') && i.complete && i.naturalWidth > 0)`, 20000, 'page 1 rendered');
	const w = await evalValue(`[...document.images].find((i) => i.src.includes('/pages/0?')).naturalWidth`);
	assert(w >= 512, 'the page is rendered at the width shown: ' + w);
	await shot('pdf-page-1');
	await key('ArrowRight', 'ArrowRight', 39);
	await waitFor(`[...document.images].some((i) => i.src.includes('/pages/1?') && i.complete && i.naturalWidth > 0)`, 20000, 'page 2 rendered');

	step('the book cover');
	const thumb = await fetch(`${BASE}/api/items/${epub.id}/thumbnail?token=${encodeURIComponent(token)}`);
	assert(thumb.status === 200 && (thumb.headers.get('content-type') ?? '').startsWith('image/'), 'a cover thumbnail: ' + thumb.status);

	const errors = consoleErrors.filter((e) => e && !/favicon/.test(e));
	assert(errors.length === 0, 'console errors:\n' + errors.join('\n'));
	console.log('\nbooks e2e: all steps passed');
} catch (err) {
	code = 1;
	console.error('\nFAILED: ' + (err instanceof Error ? err.message : err));
	if (page) await shot('failure').catch(() => {});
	if (consoleErrors.length) console.error('console:\n' + consoleErrors.join('\n'));
} finally {
	chrome.kill('SIGKILL');
	process.exit(code);
}
