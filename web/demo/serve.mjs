#!/usr/bin/env node
// Serves the demo build under its base path, for two jobs:
//
//   node demo/serve.mjs --backend http://127.0.0.1:9360 --record demo/snapshot
//     proxies /api to a real server and records every answered GET, so a
//     crawl of the client (demo/record.mjs) leaves a snapshot behind;
//
//   node demo/serve.mjs --static <dir>
//     serves an assembled demo (demo/assemble.mjs) with no backend at all,
//     as GitHub Pages will, to check it offline.
//
// Node built-ins only.
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const args = Object.fromEntries(
	process.argv.slice(2).reduce((out, a, i, all) => (a.startsWith('--') ? [...out, [a.slice(2), all[i + 1] ?? '']] : out), [])
);
const web = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const port = Number(args.port ?? 5300);
const base = args.base ?? '/lain/demo';
const root = args.static ? path.resolve(args.static) : path.join(web, 'build');
const backend = args.backend ? new URL(args.backend) : null;
const record = args.record ? path.resolve(args.record) : null;
// While recording, nothing the walk does may change the server it records.
const readOnly = 'read-only' in args || Boolean(record);

const TYPES = {
	'.html': 'text/html; charset=utf-8',
	'.js': 'text/javascript; charset=utf-8',
	'.css': 'text/css; charset=utf-8',
	'.json': 'application/json',
	'.svg': 'image/svg+xml',
	'.png': 'image/png',
	'.jpg': 'image/jpeg',
	'.webp': 'image/webp',
	'.woff2': 'font/woff2'
};

/** A request's identity in the snapshot: its path and its query without the
 * session token, sorted, so the same question always has the same name. */
export function keyOf(pathname, search) {
	const p = new URLSearchParams(search);
	p.delete('token');
	p.sort();
	const q = p.toString();
	return q ? `${pathname}?${q}` : pathname;
}

const index = new Map();
function remember(key, status, type, body) {
	if (!record || status !== 200 || !body.length) return;
	const name = crypto.createHash('sha1').update(key).digest('hex').slice(0, 16);
	fs.mkdirSync(path.join(record, 'files'), { recursive: true });
	fs.writeFileSync(path.join(record, 'files', name), body);
	index.set(key, { file: name, type });
	fs.writeFileSync(path.join(record, 'index.json'), JSON.stringify(Object.fromEntries([...index].sort()), null, '\t'));
}
if (record && fs.existsSync(path.join(record, 'index.json'))) {
	for (const [k, v] of Object.entries(JSON.parse(fs.readFileSync(path.join(record, 'index.json'), 'utf8')))) index.set(k, v);
}

function proxy(req, res, url) {
	if (readOnly && req.method !== 'GET' && req.method !== 'HEAD' && url.pathname !== '/api/auth/login') {
		req.resume();
		return res.writeHead(200, { 'content-type': 'application/json' }).end('{"status":"ok"}');
	}
	const out = http.request(
		{ host: backend.hostname, port: backend.port, path: url.pathname + url.search, method: req.method, headers: { ...req.headers, host: backend.host } },
		(up) => {
			const chunks = [];
			const keep = req.method === 'GET' && !String(up.headers['content-type'] ?? '').startsWith('text/event-stream');
			res.writeHead(up.statusCode ?? 502, up.headers);
			up.on('data', (c) => {
				if (keep) chunks.push(c);
				res.write(c);
			});
			up.on('end', () => {
				res.end();
				if (keep) remember(keyOf(url.pathname, url.search), up.statusCode, up.headers['content-type'] ?? 'application/octet-stream', Buffer.concat(chunks));
			});
		}
	);
	out.on('error', () => res.writeHead(502).end());
	req.pipe(out);
}

http
	.createServer((req, res) => {
		const url = new URL(req.url, 'http://x');
		if (backend && (url.pathname.startsWith('/api/') || url.pathname === '/health')) return proxy(req, res, url);
		if (url.pathname === base) return res.writeHead(302, { location: base + '/' }).end();
		if (!url.pathname.startsWith(base + '/')) return res.writeHead(404, { 'content-type': 'text/plain' }).end('outside the demo');
		let file = path.join(root, decodeURIComponent(url.pathname.slice(base.length + 1)) || 'index.html');
		if (!file.startsWith(root)) return res.writeHead(403).end();
		if (!fs.existsSync(file) || fs.statSync(file).isDirectory()) file = path.join(root, 'index.html');
		const headers = { 'content-type': TYPES[path.extname(file)] ?? 'application/octet-stream', 'cache-control': 'no-store' };
		// A worker at the demo's root may answer for the whole origin (the client asks for /api/...).
		if (path.basename(file) === 'sw.js') headers['service-worker-allowed'] = '/';
		res.writeHead(200, headers);
		fs.createReadStream(file).pipe(res);
	})
	.listen(port, '127.0.0.1', () => console.log(`demo on http://127.0.0.1:${port}${base}/` + (backend ? ` (api -> ${backend.origin}${record ? ', recording' : ''})` : ' (static)')));
