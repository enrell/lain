#!/usr/bin/env node
/*
 * qa-browser — the browser hands for the Lain QA fleet.
 *
 * OpenCode's integrated browser is attached to a session by the desktop
 * client, and no Linux desktop build ships yet, so a headless `just agent-e2e`
 * run cannot rely on it. This CLI drives a real Chromium over the DevTools
 * protocol with Node built-ins only (global fetch + WebSocket), the same
 * approach web/e2e/smoke.mjs uses, and keeps its state in the run directory so
 * every step of a multi-step journey stays inside one page.
 *
 *   node qa/browser.mjs start  [--headed] [--width 1440] [--height 900]
 *   node qa/browser.mjs open   URL
 *   node qa/browser.mjs goto   URL            (navigate in place, wait for load)
 *   node qa/browser.mjs snapshot [--interactive] [--max N]
 *   node qa/browser.mjs click    '@e12' | 'css selector'
 *   node qa/browser.mjs fill     '@e12' 'text'
 *   node qa/browser.mjs select   '@e12' 'option-value'
 *   node qa/browser.mjs check    '@e12' [true|false]
 *   node qa/browser.mjs press    Enter|Tab|Escape|'Control+a'
 *   node qa/browser.mjs wait     text 'Saved' | url '/settings' | load
 *   node qa/browser.mjs scroll   600            (pixels, positive is down)
 *   node qa/browser.mjs shot     [name] [--full]
 *   node qa/browser.mjs console  [--errors]
 *   node qa/browser.mjs resources [--slow N]
 *   node qa/browser.mjs contrast '@e12' | 'css'
 *   node qa/browser.mjs audit    [--json]
 *   node qa/browser.mjs eval     'JS expression returning JSON'
 *   node qa/browser.mjs localset 'lain.token' 'xyz'   (localStorage)
 *   node qa/browser.mjs status
 *   node qa/browser.mjs stop
 *
 * State lives in $QA_BROWSER_DIR (default: <repo>/qa/.browser): endpoint,
 * tab id and the ref -> selector map produced by the last snapshot.
 * Output is JSON so an agent can read it without guessing.
 */
import { spawn } from 'node:child_process';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve as pathResolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

// One browser per seed: the fleet passes --state so journeys, screenshots and
// element refs of different agents never overlap. QA_BROWSER_DIR is the
// fallback for an interactive shell that exports it once.
let STATE_DIR = process.env.QA_BROWSER_DIR || join(process.cwd(), 'qa/.browser');
const argv = process.argv.slice(2);
const verb = argv[0];

// Flags that never take a value, so `wait text Saved --timeout 4000` keeps
// "Saved" as the needle instead of swallowing the number into it.
const BOOLEAN_FLAGS = new Set(['headed', 'full', 'errors', 'interactive', 'json', 'mobile', 'cache', 'no-wait', 'new']);
const opts = {};
const pos = [];
for (let i = 1; i < argv.length; i++) {
	const token = argv[i];
	if (!token.startsWith('--')) {
		pos.push(token);
		continue;
	}
	const name = token.slice(2);
	const inline = name.indexOf('=');
	if (inline > 0) {
		opts[name.slice(0, inline)] = name.slice(inline + 1);
		continue;
	}
	if (BOOLEAN_FLAGS.has(name)) {
		opts[name] = true;
		continue;
	}
	const next = argv[i + 1];
	if (next === undefined || next.startsWith('--')) opts[name] = true;
	else {
		opts[name] = next;
		i++;
	}
}
const has = (name) => opts[name] === true;
const opt = (name, fallback) => (opts[name] === undefined || opts[name] === true ? fallback : opts[name]);
// `resolve`, not `join`: agents pass this path from a brief, and a join of two
// absolute paths silently produces `<cwd>/home/lain/...` and a leaked browser.
if (opt('state')) STATE_DIR = pathResolve(process.cwd(), String(opt('state')));

function fail(message, extra = {}) {
	console.log(JSON.stringify({ ok: false, error: message, ...extra }));
	process.exit(1);
}
function out(payload) {
	console.log(JSON.stringify({ ok: true, ...payload }));
}

/**
 * Embed an untrusted string into a page script.
 *
 * The generated script puts the value inside double quotes, but the text is
 * interpolated into a JavaScript template literal on this side, so a backtick
 * or a `${` in a selector or typed value would break out of my own source.
 * Escaping both keeps the value byte-identical after the outer template and
 * the inner string literal are evaluated.
 */
function embed(value) {
	return JSON.stringify(value)
		.replace(/`/g, '\\`')
		.replace(/\$\{/g, '\\${');
}

function stateFile() {
	return join(STATE_DIR, 'state.json');
}
function readState() {
	try {
		return JSON.parse(readFileSync(stateFile(), 'utf8'));
	} catch {
		return {};
	}
}
function writeState(patch) {
	mkdirSync(STATE_DIR, { recursive: true });
	writeFileSync(stateFile(), JSON.stringify({ ...readState(), ...patch }, null, 2));
}

async function cdpConnect() {
	const state = readState();
	if (!state.port) fail('browser is not running — run `node qa/browser.mjs start` first');
	const list = await (await fetch(`http://127.0.0.1:${state.port}/json/list`)).json();
	const want = state.tab && list.find((t) => t.id === state.tab);
	const page = want || list.find((t) => t.type === 'page') || list[0];
	if (!page?.webSocketDebuggerUrl) fail('no debuggable page', { targets: list.map((t) => t.type) });
	const ws = new WebSocket(page.webSocketDebuggerUrl);
	await new Promise((resolve, reject) => {
		ws.addEventListener('open', resolve, { once: true });
		ws.addEventListener('error', () => reject(new Error('DevTools socket failed')), { once: true });
	});
	const cdp = new Cdp(ws, page);
	// Instrument every document, current and future, so console failures and
	// page errors survive between separate CLI invocations of one journey.
	await cdp.send('Runtime.enable').catch(() => {});
	await cdp.send('Page.enable').catch(() => {});
	await cdp.send('Page.addScriptToEvaluateOnNewDocument', { source: INSTRUMENT }).catch(() => {});
	await cdp.eval(INSTRUMENT, { awaitPromise: false }).catch(() => {});
	// Element refs are handed from one CLI call to the next through state.json:
	// a full page load clears window.__qaRefs, and a journey must survive it.
	await cdp.eval(PAGE_HELPERS, { awaitPromise: false }).catch(() => {});
	if (state.refs && Object.keys(state.refs).length) {
		await cdp
			.eval(`window.__qaRefs = Object.assign(window.__qaRefs || {}, ${embed(state.refs)}); true`, { awaitPromise: false })
			.catch(() => {});
	}
	return cdp;
}

// Console and network capture installed into every document and every
// navigation, so a journey can be audited after the fact.
const INSTRUMENT = `(() => {
  if (window.__qaInstalled) return;
  window.__qaInstalled = true;
  window.__qaConsole = [];
  const push = (entry) => { window.__qaConsole.push(entry); if (window.__qaConsole.length > 200) window.__qaConsole.shift(); };
  for (const level of ['error', 'warn']) {
    const original = console[level].bind(console);
    console[level] = (...a) => {
      push({ level, text: a.map((x) => (typeof x === 'string' ? x : (() => { try { return JSON.stringify(x); } catch { return String(x); } })())).join(' ').slice(0, 500), at: Date.now() });
      original(...a);
    };
  }
  addEventListener('error', (e) => push({ level: 'error', text: String(e.message || e.error || 'window error').slice(0, 500), at: Date.now() }));
  addEventListener('unhandledrejection', (e) => push({ level: 'error', text: 'unhandledrejection: ' + String(e.reason).slice(0, 300), at: Date.now() }));
})()`;

class Cdp {
	constructor(socket, target) {
		this.socket = socket;
		this.target = target;
		this.next = 1;
		this.pending = new Map();
		socket.addEventListener('message', (ev) => {
			let msg;
			try {
				msg = JSON.parse(ev.data);
			} catch {
				return;
			}
			const call = this.pending.get(msg.id);
			if (!call) return;
			this.pending.delete(msg.id);
			msg.error ? call.reject(new Error(`${msg.error.code} ${msg.error.message}`)) : call.resolve(msg.result);
		});
	}
	send(method, params = {}) {
		const id = this.next++;
		return new Promise((resolve, reject) => {
			this.pending.set(id, { resolve, reject });
			this.socket.send(JSON.stringify({ id, method, params }));
			setTimeout(() => {
				if (this.pending.delete(id)) reject(new Error(`DevTools call timed out: ${method}`));
			}, 30000).unref?.();
		});
	}
	async eval(expression, { awaitPromise = true } = {}) {
		const res = await this.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise });
		if (res.exceptionDetails) {
			fail('page script threw', { detail: res.exceptionDetails.exception?.description || res.exceptionDetails.text });
		}
		return res.result?.value;
	}
	close() {
		try {
			this.socket.close();
		} catch {}
	}
}

/** Page-side helper: build an accessibility-ish outline with stable refs. */
const PAGE_HELPERS = `(() => {
  const win = window.__qa = window.__qa || {};
  win.cssPath = (el) => {
    if (el.id) return '#' + CSS.escape(el.id);
    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && node !== document.body) {
      const parent = node.parentElement;
      let label = node.tagName.toLowerCase();
      if (parent) {
        const same = Array.from(parent.children).filter((c) => c.tagName === node.tagName);
        if (same.length > 1) label += ':nth-of-type(' + (same.indexOf(node) + 1) + ')';
      }
      if (node.getAttribute('data-testid')) return '[data-testid="' + node.getAttribute('data-testid') + '"]';
      parts.unshift(label);
      node = parent;
      if (parts.length > 8) break;
    }
    return parts.join(' > ');
  };
  win.visible = (el) => {
    const r = el.getBoundingClientRect();
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || Number(cs.opacity) === 0) return false;
    if (el.closest('[hidden],template')) return false;
    return r.width > 1 && r.height > 1;
  };
  win.role = (el) => {
    const explicit = el.getAttribute('role');
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'button' || (tag === 'input' && ['submit','button','reset','image'].includes(type))) return 'button';
    if (tag === 'a') return el.href ? 'link' : 'generic';
    if (tag === 'input') return type === 'checkbox' ? 'checkbox' : type === 'radio' ? 'radio' : type === 'range' ? 'slider' : type === 'search' ? 'searchbox' : 'textbox';
    if (tag === 'select') return 'combobox';
    if (tag === 'textarea') return 'textbox';
    if (tag === 'video' || tag === 'audio') return 'media';
    if (tag === 'dialog') return 'dialog';
    if (/^h[1-6]$/.test(tag)) return 'heading';
    if (tag === 'nav') return 'navigation';
    if (tag === 'form') return 'form';
    if (tag === 'li') return 'listitem';
    if (tag === 'progress' || tag === 'meter') return 'progressbar';
    if (tag === 'img') return 'image';
    return '';
  };
  win.name = (el) => {
    const labelled = el.getAttribute('aria-labelledby');
    if (labelled) {
      const text = labelled.split(/\\s+/).map((id) => document.getElementById(id)?.textContent || '').join(' ').trim();
      if (text) return text;
    }
    const aria = el.getAttribute('aria-label');
    if (aria) return aria.trim();
    if (el.labels && el.labels.length) return Array.from(el.labels).map((l) => l.textContent.trim()).filter(Boolean).join(' ');
    const alt = el.getAttribute('alt');
    if (alt != null) return alt.trim();
    const title = el.getAttribute('title');
    if (title) return title.trim();
    const text = (el.textContent || '').replace(/\\s+/g, ' ').trim();
    if (text && text.length <= 80) return text;
    return (el.getAttribute('placeholder') || el.getAttribute('name') || '').trim();
  };
  win.rect = (el) => {
    const r = el.getBoundingClientRect();
    return { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) };
  };
  win.pick = (token) => {
    if (!token) return null;
    // Refs live on the page so separate CLI invocations of one journey resolve
    // the same '@e12'. SPA navigations keep the document alive, so the map
    // survives; a full load invalidates it and a fresh snapshot is required.
    const refs = window.__qaRefs || {};
    const selector = token[0] === '@' ? refs[token.slice(1)] : token;
    return selector ? document.querySelector(selector) : null;
  };
  return true;
})()`;

const SNAPSHOT = `(() => {
  const interactiveOnly = ${JSON.stringify(has('interactive'))};
  const max = ${JSON.stringify(Number(opt('max', 220)))};
  const lines = [];
  const refs = {};
  const seen = new Set();
  const focusables = 'a[href],button,input,select,textarea,summary,[contenteditable],[tabindex]:not([tabindex="-1"]),video,audio';
  let seq = 0;
  const walk = (el, depth) => {
    if (lines.length >= max || !el) return;
    const tag = el.tagName.toLowerCase();
    if (['script', 'style', 'noscript', 'template', 'svg', 'link', 'meta'].includes(tag)) return;
    const role = window.__qa.role(el);
    const name = window.__qa.name(el);
    const focusable = el.matches(focusables);
    const interesting = focusable || ['heading','navigation','dialog','form','main','media','progressbar','list','table'].includes(role);
    if (interesting && window.__qa.visible(el)) {
      const rect = window.__qa.rect(el);
      let ref = '';
      if (focusable) {
        const id = 'e' + (++seq);
        refs[id] = window.__qa.cssPath(el);
        ref = '@' + id;
      }
      const bits = [];
      if (ref) bits.push(ref);
      bits.push(role || el.tagName.toLowerCase());
      if (name) bits.push(JSON.stringify(name));
      const elId = el.id ? '#' + el.id : '';
      if (elId) bits.push(elId);
      const state = [];
      if (el.disabled) state.push('disabled');
      if (el.getAttribute('aria-expanded')) state.push('expanded=' + el.getAttribute('aria-expanded'));
      if (el.getAttribute('aria-busy') === 'true') state.push('busy');
      if (el.getAttribute('aria-invalid') === 'true') state.push('invalid');
      if (el.getAttribute('aria-current')) state.push('current=' + el.getAttribute('aria-current'));
      if (el.getAttribute('aria-checked')) state.push('checked=' + el.getAttribute('aria-checked'));
      if (el.getAttribute('aria-selected')) state.push('selected=' + el.getAttribute('aria-selected'));
      if (document.activeElement === el) state.push('focused');
      if ((el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') && el.type !== 'password') state.push('value=' + JSON.stringify(String(el.value).slice(0, 40)));
      if (el.tagName === 'INPUT' && el.type === 'password') state.push('type=password');
      if (el.tagName === 'SELECT') state.push('value=' + JSON.stringify(String(el.value)));
      if (el.tagName === 'VIDEO') {
        state.push('paused=' + el.paused);
        if (!isNaN(el.currentTime)) state.push('time=' + Math.round(el.currentTime * 10) / 10);
        if (!isNaN(el.duration) && el.duration !== Infinity) state.push('duration=' + Math.round(el.duration));
      }
      if (el.tabIndex >= 0) state.push('tabindex=' + el.tabIndex);
      bits.push('[' + Math.round(rect.x) + ',' + Math.round(rect.y) + ' ' + rect.w + 'x' + rect.h + ']');
      if (state.length) bits.push('[' + state.join(' ') + ']');
      const label = el.closest('label') && !focusable ? 'labelled' : '';
      if (label) bits.push('[' + label + ']');
      lines.push('  '.repeat(depth) + bits.join(' '));
      seen.add(el);
    }
    const nextDepth = interesting ? depth + 1 : depth;
    for (const child of el.children) walk(child, nextDepth);
    if (!interesting && el.children.length === 0) {
      const text = (el.textContent || '').replace(/\\s+/g, ' ').trim();
      if (text && lines.length < max && !interactiveOnly && !focusable && !role) {
        lines.push('  '.repeat(depth + 1) + '#text ' + JSON.stringify(text.slice(0, 90)));
      }
    }
  };
  walk(document.body, 0);
  window.__qaRefs = Object.assign(window.__qaRefs || {}, refs);
  return {
    url: location.href,
    title: document.title,
    viewport: { w: innerWidth, h: innerHeight },
    document: {
      scrollHeight: document.documentElement.scrollHeight,
      overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      lang: document.documentElement.getAttribute('lang') || null,
    },
    lines,
    refs,
    counts: { focusable: Object.keys(refs).length, truncated: lines.length >= max },
  };
})()`;

const AUDIT = `(() => {
  const issues = [];
  const add = (kind, detail, extra) => issues.push({ kind, detail, ...extra });
  const visible = (el) => window.__qa.visible(el);
  if (!document.documentElement.getAttribute('lang')) add('a11y', 'html element has no lang attribute');
  if (!document.querySelector('h1')) add('a11y', 'page has no h1 heading');
  const levels = Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6')).filter(visible);
  for (let i = 1; i < levels.length; i++) {
    const from = Number(levels[i - 1].tagName[1]);
    const to = Number(levels[i].tagName[1]);
    if (to - from > 1) add('a11y', 'heading level jumps h' + from + ' -> h' + to, { snippet: (levels[i].textContent || '').trim().slice(0, 60) });
  }
  const images = Array.from(document.querySelectorAll('img')).filter(visible);
  for (const img of images) {
    const alt = img.getAttribute('alt');
    if (alt === null) add('a11y', 'img has no alt attribute', { selector: window.__qa.cssPath(img) });
    else if (alt.trim() === '' && img.closest('a,button') && !window.__qa.name(img.closest('a,button'))) add('a11y', 'decorative alt inside a link/button with no accessible name', { selector: window.__qa.cssPath(img.closest('a,button')) });
  }
  const controls = Array.from(document.querySelectorAll('a[href],button,input,select,textarea,[role=button],[role=switch],[role=checkbox]')).filter(visible);
  for (const el of controls) {
    const name = window.__qa.name(el);
    if (!name) add('a11y', 'interactive control has no accessible name', { selector: window.__qa.cssPath(el), tag: el.tagName.toLowerCase(), type: el.type || null });
    const r = el.getBoundingClientRect();
    if (r.width < 24 || r.height < 24) add('tap', 'tap target smaller than 24x24', { selector: window.__qa.cssPath(el), w: Math.round(r.width), h: Math.round(r.height), name: name.slice(0, 40) });
  }
  const tabindex = Array.from(document.querySelectorAll('[tabindex]')).filter((el) => Number(el.getAttribute('tabindex')) > 0 && visible(el));
  for (const el of tabindex) add('a11y', 'positive tabindex changes natural focus order', { selector: window.__qa.cssPath(el), tabindex: el.getAttribute('tabindex') });
  const inputs = Array.from(document.querySelectorAll('input:not([type=hidden]),textarea,select')).filter(visible);
  for (const el of inputs) {
    if (!el.labels?.length && !el.getAttribute('aria-label') && !el.getAttribute('aria-labelledby') && !el.closest('label')) {
      add('a11y', 'form control has no label', { selector: window.__qa.cssPath(el), type: el.type || el.tagName.toLowerCase() });
    }
  }
  const overflow = document.documentElement.scrollWidth > document.documentElement.clientWidth + 1;
  if (overflow) add('layout', 'document scrolls horizontally', { scrollWidth: document.documentElement.scrollWidth, clientWidth: document.documentElement.clientWidth });
  const offscreen = Array.from(document.querySelectorAll('button,a[href],input,select')).filter((el) => {
    if (!visible(el)) return false;
    const r = el.getBoundingClientRect();
    return r.right < 0 || r.left > innerWidth;
  }).slice(0, 10).map((el) => ({ selector: window.__qa.cssPath(el), name: window.__qa.name(el).slice(0, 40) }));
  for (const item of offscreen) add('layout', 'control is horizontally off-screen', item);
  const slow = (performance.getEntriesByType('resource') || []).filter((r) => r.duration > 1500).slice(0, 10).map((r) => ({ url: r.name.replace(location.origin, ''), ms: Math.round(r.duration) }));
  for (const r of slow) add('perf', 'slow subresource', r);
  const big = (performance.getEntriesByType('resource') || []).filter((r) => (r.transferSize || 0) > 400000).slice(0, 10).map((r) => ({ url: r.name.replace(location.origin, ''), kb: Math.round((r.transferSize || 0) / 1024) }));
  for (const r of big) add('perf', 'large subresource', r);
  return { url: location.href, viewport: { w: innerWidth, h: innerHeight }, issues, totals: { controls: controls.length, images: images.length } };
})()`;

function parseColor(value) {
	if (!value) return null;
	const m = value.match(/rgba?\(([\d.]+),\s*([\d.]+),\s*([\d.]+)(?:,\s*([\d.]+))?\)/);
	if (!m) return null;
	return { r: +m[1], g: +m[2], b: +m[3], a: m[4] === undefined ? 1 : +m[4] };
}

const CONTRAST = `(() => {
  const el = window.__qa.pick(${embed(pos[0] || '')});
  if (!el) return { error: 'token did not resolve to an element' };
  const stack = [];
  let node = el;
  while (node && node !== document.documentElement) {
    const bg = getComputedStyle(node).backgroundColor;
    stack.push({ selector: window.__qa.cssPath(node), bg });
    node = node.parentElement;
  }
  const cs = getComputedStyle(el);
  return {
    name: (el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 60),
    color: cs.color,
    fontSize: cs.fontSize,
    fontWeight: cs.fontWeight,
    backgroundStack: stack.slice(0, 6),
    rect: window.__qa.rect(el),
  };
})()`;

const commands = {
	async start() {
		// Idempotent on purpose: an agent that calls `start` twice must not leak
		// a second Chromium behind a stale state file.
		const alive = readState();
		if (alive.port && !has('new')) {
			try {
				const version = await (await fetch(`http://127.0.0.1:${alive.port}/json/version`)).json();
				return out({ port: alive.port, browser: version.Browser, headless: alive.headless, state: STATE_DIR, reused: true });
			} catch {
				/* state file points at a dead browser: start a fresh one below */
			}
		}
		const port = await freePort();
		const profile = opt('profile', join(tmpdir(), `lain-qa-chromium-${port}`));
		rmSync(profile, { recursive: true, force: true });
		mkdirSync(profile, { recursive: true });
		const binary = process.env.LAIN_AGENT_CHROMIUM || 'chromium';
		const args = [
			'--disable-gpu',
			'--no-first-run',
			'--no-default-browser-check',
			'--disable-dev-shm-usage',
			'--mute-audio',
			'--autoplay-policy=no-user-gesture-required',
			'--disable-features=DialMediaRouteProvider',
			`--remote-debugging-port=${port}`,
			`--user-data-dir=${profile}`,
			`--window-size=${opt('width', 1440)},${opt('height', 900)}`,
		];
		if (!has('headed')) args.unshift('--headless=new');
		const child = spawn(binary, args, { stdio: 'ignore', detached: true });
		child.unref();
		let ready = null;
		for (let i = 0; i < 80; i++) {
			try {
				ready = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
				break;
			} catch {
				await sleep(250);
			}
		}
		if (!ready) {
			child.kill('SIGKILL');
			fail(`${binary} did not start a DevTools endpoint on 127.0.0.1:${port}`, { profile });
		}
		writeState({ port, pid: child.pid, profile, refs: {}, headless: !has('headed'), browser: ready.Browser });
		out({ port, browser: ready.Browser, headless: !has('headed'), state: STATE_DIR });
	},

	async status() {
		const state = readState();
		if (!state.port) return out({ running: false });
		try {
			const list = await (await fetch(`http://127.0.0.1:${state.port}/json/list`)).json();
			return out({ running: true, port: state.port, headless: state.headless, tabs: list.filter((t) => t.type === 'page').map((t) => ({ id: t.id, url: t.url, title: t.title })) });
		} catch (err) {
			return out({ running: false, reason: String(err.message || err) });
		}
	},

	async stop() {
		const state = readState();
		try {
			process.kill(state.pid, 'SIGTERM');
		} catch {}
		rmSync(state.profile, { recursive: true, force: true });
		rmSync(stateFile(), { force: true });
		out({ stopped: state.pid ?? null });
	},

	async open() {
		const url = pos[0] || opt('url');
		if (!url) fail('open needs a URL');
		const state = readState();
		if (!state.port) fail('browser is not running — run `node qa/browser.mjs start` first');
		const created = await (await fetch(`http://127.0.0.1:${state.port}/json/new?${encodeURIComponent(url)}`, { method: 'PUT' })).json();
		writeState({ tab: created.id, refs: {} });
		const cdp = await cdpConnect();
		await cdp.send('Page.enable');
		await cdp.send('Runtime.enable');
		await waitForLoad(cdp);
		const page = await pageSummary(cdp);
		cdp.close();
		out({ tab: created.id, ...page });
	},

	async goto() {
		const url = pos[0] || opt('url');
		if (!url) fail('goto needs a URL');
		const cdp = await cdpConnect();
		await cdp.send('Page.enable');
		await cdp.send('Page.navigate', { url });
		await waitForLoad(cdp);
		const page = await pageSummary(cdp);
		cdp.close();
		writeState({ refs: {} });
		out(page);
	},

	async reload() {
		const cdp = await cdpConnect();
		await cdp.send('Page.enable');
		await cdp.send('Page.reload', { ignoreCache: has('cache') });
		await waitForLoad(cdp);
		const page = await pageSummary(cdp);
		cdp.close();
		out(page);
	},

	async snapshot() {
		const cdp = await cdpConnect();
		await cdp.send('Runtime.enable');
		await cdp.eval(PAGE_HELPERS, { awaitPromise: false });
		const snap = await cdp.eval(SNAPSHOT);
		const state = readState();
		writeState({ refs: { ...(state.refs || {}), ...snap.refs } });
		cdp.close();
		const body = snap.lines.join('\n');
		if (has('json')) return out(snap);
		console.log(
			[
				`${snap.title} — ${snap.url}`,
				`viewport ${snap.viewport.w}x${snap.viewport.h} · doc height ${snap.document.scrollHeight}${snap.document.overflowX ? ' · HORIZONTAL OVERFLOW' : ''} · lang=${snap.document.lang ?? 'MISSING'}`,
				'',
				body,
				snap.counts.truncated ? `\n(truncated at ${snap.lines.length} nodes; pass --max or --interactive to narrow)` : '',
			]
				.filter(Boolean)
				.join('\n')
		);
		process.exit(0);
	},

	async click() {
		const token = pos[0];
		if (!token) fail('click needs @ref or a css selector');
		const cdp = await cdpConnect();
		await cdp.eval(PAGE_HELPERS, { awaitPromise: false });
		const box = await resolve(cdp, token);
		await clickPoint(cdp, box);
		await sleep(has('no-wait') ? 0 : 400);
		out({ clicked: box, ...(await pageSummary(cdp)) });
		cdp.close();
	},

	async fill() {
		const [token, ...rest] = pos;
		const value = String(opt('value', rest.join(' ')) ?? '');
		if (!token) fail('fill needs @ref or a css selector');
		const cdp = await cdpConnect();
		await cdp.eval(PAGE_HELPERS, { awaitPromise: false });
		const box = await resolve(cdp, token);
		// Focus with a real click: setting .value from script skips the handlers
		// that decide whether the app even enables the submit button, so the
		// audit would measure a form the user can never reach.
		await clickPoint(cdp, box);
		await key(cdp, { type: 'keyDown', key: 'a', code: 'KeyA', modifiers: 2, windowsVirtualKeyCode: 65 });
		await key(cdp, { type: 'keyUp', key: 'a', code: 'KeyA', modifiers: 2, windowsVirtualKeyCode: 65 });
		await key(cdp, { type: 'keyDown', key: 'Backspace', code: 'Backspace', windowsVirtualKeyCode: 8 });
		await key(cdp, { type: 'keyUp', key: 'Backspace', code: 'Backspace', windowsVirtualKeyCode: 8 });
		let strategy = 'insertText';
		if (value) {
			await cdp.send('Input.insertText', { text: value }).catch(() => {});
			if (!(await readValue(cdp, token)).endsWith(value.slice(-8))) {
				strategy = 'keyEvents';
				for (const ch of value) {
					await key(cdp, { type: 'keyDown', key: ch, text: ch, unmodifiedText: ch });
					await key(cdp, { type: 'keyUp', key: ch });
				}
			}
		}
		await sleep(250);
		const after = await cdp.eval(`(() => { const el = window.__qa.pick(${embed(token)}); return el ? { value: el.value, checked: el.checked } : null; })()`);
		out({ filled: box, typed: value.length, strategy, state: after });
		cdp.close();
	},

	async select() {
		const [token, value] = pos;
		if (!token) fail('select needs @ref or a css selector and a value');
		const cdp = await cdpConnect();
		await cdp.eval(PAGE_HELPERS, { awaitPromise: false });
		const applied = await cdp.eval(
			`(() => { const el = window.__qa.pick(${embed(token)}); if (!el) return null; el.value = ${embed(value ?? '')}; el.dispatchEvent(new Event('input', {bubbles:true})); el.dispatchEvent(new Event('change', {bubbles:true})); return el.value; })()`
		);
		if (applied === null) fail('token did not resolve');
		await sleep(300);
		out({ selected: applied });
		cdp.close();
	},

	async check() {
		const [token, want] = pos;
		if (!token) fail('check needs @ref or a css selector');
		const cdp = await cdpConnect();
		await cdp.eval(PAGE_HELPERS, { awaitPromise: false });
		const result = await cdp.eval(
			`(() => { const el = window.__qa.pick(${embed(token)}); if (!el) return null; const target = ${embed(want !== 'false')}; if (el.checked !== target) el.click(); return { checked: el.checked, role: el.getAttribute('role') }; })()`
		);
		if (result === null) fail('token did not resolve');
		await sleep(250);
		out(result);
		cdp.close();
	},

	async press() {
		const key = pos[0] || opt('key', 'Enter');
		const cdp = await cdpConnect();
		const map = { Enter: 13, Escape: 27, Tab: 9, ArrowDown: 40, ArrowUp: 38, ArrowLeft: 37, ArrowRight: 39, Backspace: 8, ' ': 32, Space: 32, Home: 36, End: 35 };
		const code = { Enter: 'Enter', Escape: 'Escape', Tab: 'Tab', ArrowDown: 'ArrowDown', ArrowUp: 'ArrowUp', ArrowLeft: 'ArrowLeft', ArrowRight: 'ArrowRight', Backspace: 'Backspace', ' ': 'Space', Space: 'Space', Home: 'Home', End: 'End' }[key] || key;
		const vk = map[key] ?? 0;
		await cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key, code, windowsVirtualKeyCode: vk });
		if (key === 'Enter' || key === ' ') await cdp.send('Input.dispatchKeyEvent', { type: 'char', key, text: key === ' ' ? ' ' : '\r' });
		await cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode: vk });
		await sleep(400);
		out({ pressed: key, ...(await pageSummary(cdp)) });
		cdp.close();
	},

	async scroll() {
		const y = Number(pos[0] ?? opt('y', 600));
		const cdp = await cdpConnect();
		await cdp.eval(`window.scrollBy(0, ${y}); true`, { awaitPromise: false });
		await sleep(300);
		out(await pageSummary(cdp));
		cdp.close();
	},

	async wait() {
		const mode = pos[0] || 'load';
		const needle = pos.slice(1).join(' ');
		const cdp = await cdpConnect();
		const budget = Number(opt('timeout', 15000));
		const started = Date.now();
		let ok = false;
		let last = null;
		while (Date.now() - started < budget) {
			if (mode === 'text') {
				last = await cdp.eval(`document.body?.innerText || ''`, { awaitPromise: false });
				ok = String(last).includes(needle);
			} else if (mode === 'url') {
				last = await cdp.eval('location.href', { awaitPromise: false });
				ok = String(last).includes(needle);
			} else if (mode === 'gone') {
				last = await cdp.eval(`document.body?.innerText || ''`, { awaitPromise: false });
				ok = !String(last).includes(needle);
			} else {
				await waitForLoad(cdp, { quick: true });
				ok = true;
			}
			if (ok) break;
			await sleep(200);
		}
		const page = await pageSummary(cdp);
		cdp.close();
		if (!ok) fail(`wait ${mode} "${needle}" timed out after ${budget}ms`, { current: page.url, sample: typeof last === 'string' ? last.slice(0, 200) : last });
		out({ waited: mode, needle, waitedMs: Date.now() - started, ...page });
	},

	async shot() {
		const name = pos[0] || `shot-${Date.now()}`;
		const cdp = await cdpConnect();
		await cdp.send('Page.enable');
		const params = { format: 'png' };
		if (has('full')) {
			const height = await cdp.eval('Math.max(document.documentElement.scrollHeight, 1)', { awaitPromise: false });
			params.captureBeyondViewport = true;
			params.clip = { x: 0, y: 0, width: 1440, height: Math.min(Number(height) || 900, 8000), scale: 1 };
		}
		const shot = await cdp.send('Page.captureScreenshot', params);
		const file = join(STATE_DIR, 'shots', `${String(name).replace(/[^a-z0-9._-]/gi, '_')}.png`);
		mkdirSync(join(STATE_DIR, 'shots'), { recursive: true });
		writeFileSync(file, Buffer.from(shot.data, 'base64'));
		cdp.close();
		out({ file, bytes: readFileSync(file).byteLength });
	},

	async console() {
		const cdp = await cdpConnect();
		const entries = await cdp.eval('window.__qaConsole || []', { awaitPromise: false });
		const errors = has('errors') ? (entries || []).filter((e) => e.level === 'error') : entries || [];
		cdp.close();
		out({ count: errors.length, entries: errors.slice(-40).map((e) => ({ level: e.level, text: String(e.text).slice(0, 400) })) });
	},

	async resources() {
		const cdp = await cdpConnect();
		const list = await cdp.eval(
			`(performance.getEntriesByType('resource') || []).map((r) => ({ url: r.name, kind: r.initiatorType, ms: Math.round(r.duration), bytes: r.transferSize || 0, status: r.responseStatus ?? null })).filter((r) => r.ms > ${Number(opt('slow', 0))} || r.status >= 400)`,
			{ awaitPromise: false }
		);
		cdp.close();
		out({ count: (list || []).length, resources: list });
	},

	async contrast() {
		const cdp = await cdpConnect();
		await cdp.eval(PAGE_HELPERS, { awaitPromise: false });
		const raw = await cdp.eval(CONTRAST);
		cdp.close();
		if (raw?.error) fail(raw.error);
		const fg = parseColor(raw.color);
		let bg = null;
		for (const layer of raw.backgroundStack || []) {
			const c = parseColor(layer.bg);
			if (c && c.a > 0.02) {
				bg = c;
				break;
			}
		}
		if (!fg || !bg) return out({ ...raw, ratio: null, note: 'could not resolve opaque colours; treat as unverified' });
		const mix = (a, b) => ({ r: a.r * a.a + b.r * (1 - a.a), g: a.g * a.a + b.g * (1 - a.a), b: a.b * a.a + b.b * (1 - a.a) });
		const blended = mix(fg, bg);
		const lum = ({ r, g, b }) => {
			const ch = [r, g, b].map((v) => {
				const c = v / 255;
				return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
			});
			return 0.2126 * ch[0] + 0.7152 * ch[1] + 0.0722 * ch[2];
		};
		const l1 = lum(blended);
		const l2 = lum(bg);
		const ratio = (Math.max(l1, l2) + 0.05) / (Math.min(l1, l2) + 0.05);
		const large = Number.parseFloat(raw.fontSize) >= 24 || (Number.parseFloat(raw.fontSize) >= 18.66 && Number(raw.fontWeight) >= 700);
		const required = large ? 3 : 4.5;
		out({ ...raw, ratio: Number(ratio.toFixed(2)), wcag_required: required, pass: ratio >= required });
	},

	async audit() {
		const cdp = await cdpConnect();
		await cdp.eval(PAGE_HELPERS, { awaitPromise: false });
		const result = await cdp.eval(AUDIT);
		const errors = await cdp.eval('window.__qaConsole || []', { awaitPromise: false });
		cdp.close();
		const payload = { ...result, console_errors: (errors || []).filter((e) => e.level === 'error').map((e) => String(e.text).slice(0, 300)) };
		if (has('json')) return out(payload);
		if (!payload.issues.length && !payload.console_errors.length) {
			console.log(`no automatic findings on ${payload.url}`);
			process.exit(0);
		}
		console.log(
			[
				`automatic checks on ${payload.url} (viewport ${payload.viewport.w}x${payload.viewport.h})`,
				...payload.issues.map((i) => `- [${i.kind}] ${i.detail}${i.selector ? ` :: ${i.selector}` : ''}${i.name ? ` "${i.name}"` : ''}${i.ms ? ` ${i.ms}ms` : ''}${i.url ? ` ${i.url}` : ''}`),
				...payload.console_errors.map((e) => `- [console] ${e}`),
			].join('\n')
		);
		process.exit(0);
	},

	async eval() {
		const script = pos.join(' ') || opt('script');
		if (!script) fail('eval needs a JS expression');
		const cdp = await cdpConnect();
		const value = await cdp.eval(`(async () => { const r = await (${script}); return typeof r === 'string' ? r : JSON.parse(JSON.stringify(r ?? null)); })()`);
		cdp.close();
		out({ result: value });
	},

	async viewport() {
		const width = Number(pos[0] || opt('width', 1280));
		const height = Number(pos[1] || opt('height', 800));
		const state = readState();
		const cdp = await cdpConnect();
		await cdp.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: has('mobile') });
		await sleep(400);
		cdp.close();
		out({ viewport: { width, height, mobile: has('mobile') }, state: state.port });
	},

	async localset() {
		const [key, value] = pos;
		if (!key) fail('localset needs a key and value');
		const cdp = await cdpConnect();
		await cdp.eval(`localStorage.setItem(${embed(key)}, ${embed(value ?? '')}); true`, { awaitPromise: false });
		cdp.close();
		out({ key, set: true });
	},

	async cookie() {
		const cdp = await cdpConnect();
		const cookies = await cdp.send('Network.getCookies');
		cdp.close();
		out({ cookies: cookies.cookies.map((c) => ({ name: c.name, domain: c.domain, value: String(c.value).slice(0, 12) })) });
	},
};

async function key(cdp, params) {
	await cdp.send('Input.dispatchKeyEvent', params);
}

async function clickPoint(cdp, box) {
	await sleep(60);
	for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
		await cdp.send('Input.dispatchMouseEvent', {
			type,
			x: box.cx,
			y: box.cy,
			button: 'left',
			clickCount: type === 'mouseMoved' ? 0 : 1,
			buttons: type === 'mousePressed' ? 1 : 0,
		});
	}
	await sleep(120);
}

async function readValue(cdp, token) {
	return String(await cdp.eval(`(() => { const el = window.__qa.pick(${embed(token)}); return el && el.value != null ? String(el.value) : ''; })()`) || '');
}

async function resolve(cdp, token) {
	const quoted = embed(token);
	const fallback = token.startsWith('@') ? 'null' : `document.querySelector(${quoted})`;
	const probe = `(() => {
    window.__qa = window.__qa || {};
    const el = (window.__qa.pick ? window.__qa.pick(${quoted}) : null) || (${fallback});
    if (!el) return { error: 'no element for ' + ${quoted} + '; take a fresh snapshot' };
    el.scrollIntoView({ block: 'center', inline: 'center' });
    const r = el.getBoundingClientRect();
    return { cx: Math.round(r.x + r.width / 2), cy: Math.round(r.y + r.height / 2), w: Math.round(r.width), h: Math.round(r.height), text: (el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 60) };
  })()`;
	const box = await cdp.eval(probe);
	if (box?.error) fail(box.error);
	return box;
}

async function waitForLoad(cdp, { quick = false } = {}) {
	for (let i = 0; i < (quick ? 20 : 120); i++) {
		const ready = await cdp.eval('document.readyState', { awaitPromise: false }).catch(() => null);
		if (ready === 'complete' || ready === 'interactive') break;
		await sleep(150);
	}
	await sleep(quick ? 150 : 700);
}

async function pageSummary(cdp) {
	return await cdp.eval(
		`({
      url: location.href,
      title: document.title,
      scrollY: Math.round(window.scrollY),
      docHeight: document.documentElement.scrollHeight,
      overflowX: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      busy: !!document.querySelector('[aria-busy=true],.animate-pulse'),
      text: (document.body?.innerText || '').replace(/\\s+/g, ' ').trim().slice(0, 220),
    })`,
		{ awaitPromise: false }
	);
}

function freePort() {
	return new Promise((resolve, reject) => {
		const srv = createServer();
		srv.on('error', reject);
		srv.listen(0, '127.0.0.1', () => {
			const { port } = srv.address();
			srv.close(() => resolve(port));
		});
	});
}

if (!verb || !commands[verb]) fail(`unknown verb "${verb}"`, { verbs: Object.keys(commands) });
try {
	await commands[verb]();
} catch (err) {
	fail(String(err?.message || err));
}
