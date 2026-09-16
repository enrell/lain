/*
 * Disposable target instance for the QA fleet.
 *
 * Auditors must never point at a contributor's real library: they click
 * through settings, delete things they created, and write progress. So every
 * run gets a throwaway data directory, the synthetic fixtures from
 * web/e2e/fixtures.sh, and credentials the fleet knows in advance. The
 * instance is stopped and wiped unless LAIN_AGENT_KEEP_INSTANCE=1.
 */
import { execFile, spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { existsSync, readdirSync, statSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const CACHE = 'qa/.cache';

export class InstanceError extends Error {}

/*
 * A crash in the runner (an uncaught bug, a killed parent) skips every
 * stopInstance on the normal path, orphaning the disposable servers it spawned.
 * Each live child is tracked here and killed on exit, so an abnormal end still
 * leaves no `lain serve` behind. A child leaves the set only when it actually
 * exits: stop() signals SIGTERM for a graceful shutdown, but a SIGTERM that the
 * server does not act on (it can wedge while its data dir is being removed) is
 * then finished by the exit handler's SIGKILL instead of being lost.
 */
const liveChildren = new Set();
let exitCleanupInstalled = false;
function installExitCleanup() {
	if (exitCleanupInstalled) return;
	exitCleanupInstalled = true;
	process.on('exit', () => {
		for (const child of liveChildren) {
			try {
				child.kill('SIGKILL');
			} catch {}
		}
	});
}

export function freePort() {
	return new Promise((resolve, reject) => {
		const srv = createServer();
		srv.on('error', reject);
		srv.listen(0, '127.0.0.1', () => {
			const { port } = srv.address();
			srv.close(() => resolve(port));
		});
	});
}

function newestMtime(dir) {
	let newest = 0;
	let name = null;
	const stack = [dir];
	while (stack.length) {
		const current = stack.pop();
		let entries;
		try {
			entries = readdirSync(current, { withFileTypes: true });
		} catch {
			continue;
		}
		for (const entry of entries) {
			if (entry.name.startsWith('.') || entry.name === 'node_modules') continue;
			const full = join(current, entry.name);
			if (entry.isDirectory()) stack.push(full);
			else {
				const m = statSync(full).mtimeMs;
				if (m > newest) {
					newest = m;
					name = full;
				}
			}
		}
	}
	return { newest, name };
}

function run(command, args, { cwd, timeout = 600000, env = {} } = {}) {
	return new Promise((resolve, reject) => {
		const child = spawn(command, args, { cwd, env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
		let out = '';
		let err = '';
		const timer = setTimeout(() => {
			child.kill('SIGKILL');
			reject(new InstanceError(`${command} ${args.join(' ')} timed out`));
		}, timeout);
		child.stdout.on('data', (b) => (out += b.toString()));
		child.stderr.on('data', (b) => (err += b.toString()));
		child.on('error', (e) => {
			clearTimeout(timer);
			reject(new InstanceError(`${command} is not available: ${e.message}`));
		});
		child.on('close', (code) => {
			clearTimeout(timer);
			if (code === 0) resolve({ out, err });
			else reject(new InstanceError(`${command} ${args.join(' ')} failed (${code})\n${err.slice(-4000) || out.slice(-4000)}`));
		});
	});
}

/** Rebuild the embedded SPA whenever the sources are newer than the bundle. */
async function ensureWeb(root, log) {
	const dist = join(root, 'internal/webui/dist/index.html');
	const src = newestMtime(join(root, 'web/src'));
	if (existsSync(dist) && statSync(dist).mtimeMs >= src.newest) {
		log(`web bundle is current (${dist})`);
		return;
	}
	log('web bundle is stale for the QA target: rebuilding (this also refreshes the showroom build)');
	await run('just', ['web'], { cwd: root, timeout: 900000 });
	if (!existsSync(dist)) throw new InstanceError(`just web did not produce ${dist}`);
}

async function ensureBinary(root, log) {
	const binary = join(root, CACHE, 'lain');
	const src = newestMtime(join(root, 'internal'));
	const goSrc = newestMtime(join(root, 'cmd'));
	const newest = Math.max(src.newest, goSrc.newest);
	if (existsSync(binary) && statSync(binary).mtimeMs >= newest) return binary;
	log('building the QA target binary');
	await run('go', ['build', '-trimpath', '-o', join(CACHE, 'lain'), './cmd/lain'], { cwd: root, timeout: 600000 });
	return binary;
}

async function ensureFixtures(root, log) {
	const dir = join(root, CACHE, 'fixtures');
	if (existsSync(join(dir, 'Anime/Frieren/Frieren - 01.webm'))) return dir;
	log('generating synthetic media fixtures (ffmpeg)');
	await run('bash', ['web/e2e/fixtures.sh', dir], { cwd: root, timeout: 600000 });
	return dir;
}

async function api(base, path, { method = 'GET', token, body } = {}) {
	const res = await fetch(`${base}${path}`, {
		method,
		headers: {
			'content-type': 'application/json',
			...(token ? { authorization: `Bearer ${token}` } : {}),
		},
		body: body === undefined ? undefined : JSON.stringify(body),
	});
	const text = await res.text();
	let json = null;
	try {
		json = text ? JSON.parse(text) : null;
	} catch {
		/* non-JSON body: surfaced below */
	}
	if (!res.ok) {
		throw new InstanceError(`${method} ${path} -> ${res.status}: ${text.slice(0, 400)}`);
	}
	return json;
}

/**
 * Boot an isolated instance, seed accounts, libraries and a finished scan.
 * @returns {Promise<{base:string,pid:number,stop:()=>void,secrets:object}>}
 */
export async function startInstance(root, { runId, log = () => {}, external, bare = false } = {}) {
	if (external) {
		const base = external.replace(/\/$/, '');
		let health = null;
		for (let attempt = 0; attempt < 10; attempt++) {
			try {
				health = await (await fetch(`${base}/api/health`)).json();
				break;
			} catch {
				await sleep(400);
			}
		}
		if (!health) throw new InstanceError(`external target ${base} is not answering /api/health`);
		return { base, external: true, pid: null, stop: () => {}, secrets: {}, health };
	}

	await ensureWeb(root, log);
	const binary = await ensureBinary(root, log);
	const fixtures = await ensureFixtures(root, log);
	const dataDir = join(root, CACHE, `data-${runId}`);
	rmSync(dataDir, { recursive: true, force: true });
	const port = await freePort();
	const base = `http://127.0.0.1:${port}`;
	log(`starting disposable instance on ${base} (data ${dataDir})`);

	const child = spawn(binary, ['serve', '--data-dir', dataDir, '--port', String(port), '--bind', '127.0.0.1'], {
		cwd: root,
		env: { ...process.env, LAIN_LOG_LEVEL: process.env.LAIN_LOG_LEVEL || 'warn' },
		stdio: ['ignore', 'pipe', 'pipe'],
	});
	liveChildren.add(child);
	child.on('exit', () => liveChildren.delete(child));
	installExitCleanup();
	let serverLog = '';
	child.stdout.on('data', (b) => (serverLog += b.toString()));
	child.stderr.on('data', (b) => (serverLog += b.toString()));
	child.on('exit', (code) => log(`QA instance exited with code ${code}`));

	const stop = () => {
		try {
			child.kill('SIGTERM');
		} catch {}
		if (!process.env.LAIN_AGENT_KEEP_INSTANCE || process.env.LAIN_AGENT_KEEP_INSTANCE === '0') {
			rmSync(dataDir, { recursive: true, force: true });
		}
	};

	let health = null;
	for (let attempt = 0; attempt < 60; attempt++) {
		try {
			health = await (await fetch(`${base}/api/health`)).json();
			break;
		} catch {
			if (child.exitCode !== null) break;
			await sleep(250);
		}
	}
	if (!health) throw new InstanceError(`QA instance never became healthy:\n${serverLog.slice(-2000)}`);

	if (bare) {
		// A bare instance answers /api/health and nothing else: no accounts, no
		// libraries. `web/e2e/smoke.mjs` is a first-run journey that creates the
		// admin itself, so the deterministic gate needs a virgin server — pointed
		// at the already-seeded one it fails on the setup screen.
		return { base, pid: child.pid, stop, fixtures, bare: true, secrets: {}, health: { setup_required: true } };
	}

	const admin = {
		username: process.env.LAIN_AGENT_ADMIN_USER || 'qa-admin',
		password: process.env.LAIN_AGENT_ADMIN_PASSWORD || `Admin-${runId}-pw`,
	};
	const member = {
		username: process.env.LAIN_AGENT_MEMBER_USER || 'qa-member',
		password: process.env.LAIN_AGENT_MEMBER_PASSWORD || `Member-${runId}-pw`,
	};

	const status = await api(base, '/api/setup/status');
	if (status.setup_required) {
		await api(base, '/api/setup', { method: 'POST', body: admin });
	}
	const login = await api(base, '/api/auth/login', { method: 'POST', body: admin });
	const token = login.token;

	const libraries = await api(base, '/api/libraries', { token });
	const wanted = [
		{ name: 'Anime', type: 'anime', path: join(fixtures, 'Anime') },
		{ name: 'Movies', type: 'movie', path: join(fixtures, 'Movies') },
	];
	for (const lib of wanted) {
		if ((libraries || []).some((existing) => existing.path === lib.path)) continue;
		await api(base, '/api/libraries', { method: 'POST', token, body: lib });
	}
	// The list endpoint returns a bare array; re-read it so the seed report
	// names the libraries the auditors are about to find in the UI.
	const seededLibraries = (await api(base, '/api/libraries', { token })) || [];

	await api(base, '/api/library/scan', { method: 'POST', token });
	let scan = null;
	for (let attempt = 0; attempt < 160; attempt++) {
		scan = await api(base, '/api/library/scan', { token });
		if (scan.state !== 'running') break;
		await sleep(500);
	}
	if (scan?.state !== 'done') throw new InstanceError(`seed scan did not finish: ${JSON.stringify(scan)}`);
	if (scan.stats?.errors) log(`seed scan reported ${scan.stats.errors} error(s) — auditors will see them too`);

	// A second, non-admin account so gating and "what may a member see"
	// journeys are auditable without pretending to be an attacker.
	let memberLogin = null;
	try {
		await api(base, '/api/users', { method: 'POST', token, body: { ...member, role: 'user' } });
		memberLogin = await api(base, '/api/auth/login', { method: 'POST', body: member });
	} catch (err) {
		log(`member account already present (${String(err.message).slice(0, 80)})`);
		memberLogin = await api(base, '/api/auth/login', { method: 'POST', body: member }).catch(() => null);
	}

	const catalog = await api(base, '/api/catalog?limit=200', { token });
	return {
		base,
		pid: child.pid,
		stop,
		health,
		fixtures,
		secrets: {
			admin: { ...admin, token },
			member: memberLogin ? { ...member, token: memberLogin.token } : null,
		},
		seed: {
			scan: scan.stats,
			catalog_count: (catalog.items || []).length,
			libraries: seededLibraries.map((lib) => ({ id: lib.id, name: lib.name, type: lib.type, path: lib.path })),
			sample_ids: (catalog.items || []).slice(0, 5).map((item) => ({ id: item.id, title: item.title, kind: item.kind })),
			catalog_summary: `${(catalog.items || []).length} catalog item(s) from ${scan.stats?.identified ?? '?'} identified file(s); media root ${fixtures}`,
			media_root: fixtures,
		},
	};
}

/**
 * A tangled agent can start its own detached server (`setsid … &`) to look at
 * the branch tip; `liveChildren` never sees it, so it outlives the round. Once
 * the runner has stopped its own targets, anything still serving this run's
 * data dir is a stray: kill it and return the pids so the caller can say so.
 * Best-effort and Linux-only; the pattern is the run's own data dir, so a real
 * contributor server pointed anywhere else is never matched.
 */
export function killStrayTargets(runId) {
	return new Promise((resolve) => {
		execFile('pgrep', ['-f', `lain serve --data-dir [^ ]*data-${runId} `], (err, stdout) => {
			const pids = String(stdout || '')
				.split('\n')
				.map((line) => Number(line.trim()))
				.filter(Boolean);
			for (const pid of pids) {
				try {
					process.kill(pid, 'SIGKILL');
				} catch {}
			}
			resolve(pids);
		});
	});
}
