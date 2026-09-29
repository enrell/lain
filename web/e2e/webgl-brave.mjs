#!/usr/bin/env node
/*
 * Procedural WebGL2 Anime4K probe driven through real Brave.
 *
 * Starts the source Vite server, drives Brave through CDP, creates a
 * same-origin synthetic video with canvas.captureStream(), imports the
 * production WebGL engine and reports which Anime4K modes present a frame.
 * No Lain server, credentials or media files are involved.
 *
 *   node e2e/webgl-brave.mjs [--brave brave] [--mode anime4k-a]
 */
import { spawn } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const args = process.argv.slice(2);
const arg = (name, fallback) => {
	const index = args.indexOf(`--${name}`);
	return index >= 0 && args[index + 1] ? args[index + 1] : fallback;
};
const BRAVE = arg('brave', 'brave');
const ONLY_MODE = arg('mode', '');

async function freePort() {
	const server = createServer();
	await new Promise((resolve, reject) => {
		server.once('error', reject);
		server.listen(0, '127.0.0.1', resolve);
	});
	const address = server.address();
	const port = typeof address === 'object' && address ? address.port : 0;
	await new Promise((resolve) => server.close(resolve));
	if (!port) throw new Error('could not allocate a local port');
	return port;
}

async function waitHTTP(url, label) {
	const deadline = Date.now() + 20_000;
	while (Date.now() < deadline) {
		try {
			const response = await fetch(url);
			if (response.ok) return;
		} catch {
			// Process is still starting.
		}
		await sleep(100);
	}
	throw new Error(`${label} did not become ready`);
}

class CDP {
	constructor(socket) {
		this.socket = socket;
		this.nextID = 0;
		this.pending = new Map();
		socket.addEventListener('message', (event) => {
			const message = JSON.parse(event.data);
			if (message.id === undefined) return;
			const pending = this.pending.get(message.id);
			if (!pending) return;
			this.pending.delete(message.id);
			if (message.error) pending.reject(new Error(message.error.message));
			else pending.resolve(message.result);
		});
	}

	static async connect(url) {
		const socket = new WebSocket(url);
		await new Promise((resolve, reject) => {
			socket.addEventListener('open', resolve, { once: true });
			socket.addEventListener('error', () => reject(new Error('CDP websocket failed')), {
				once: true
			});
		});
		return new CDP(socket);
	}

	send(method, params = {}) {
		const id = ++this.nextID;
		return new Promise((resolve, reject) => {
			this.pending.set(id, { resolve, reject });
			this.socket.send(JSON.stringify({ id, method, params }));
		});
	}

	close() {
		this.socket.close();
	}
}

const vitePort = await freePort();
const debugPort = await freePort();
const profile = mkdtempSync(join(tmpdir(), 'lain-webgl-brave-'));
const vite = spawn('pnpm', ['dev', '--host', '127.0.0.1', '--port', String(vitePort)], {
	cwd: new URL('..', import.meta.url),
	stdio: 'ignore'
});
const brave = spawn(
	BRAVE,
	[
		'--headless=new',
		'--no-first-run',
		'--no-default-browser-check',
		'--disable-dev-shm-usage',
		'--disable-gpu-sandbox',
		'--use-angle=swiftshader',
		`--remote-debugging-port=${debugPort}`,
		`--user-data-dir=${profile}`,
		'about:blank'
	],
	{ stdio: 'ignore' }
);

let page;
try {
	await Promise.all([
		waitHTTP(`http://127.0.0.1:${vitePort}/`, 'Vite'),
		waitHTTP(`http://127.0.0.1:${debugPort}/json/version`, 'Brave DevTools')
	]);
	const target = await (
		await fetch(
			`http://127.0.0.1:${debugPort}/json/new?${encodeURIComponent(`http://127.0.0.1:${vitePort}/login`)}`,
			{ method: 'PUT' }
		)
	).json();
	page = await CDP.connect(target.webSocketDebuggerUrl);
	await page.send('Runtime.enable');
	await sleep(500);

	const expression = `(async () => {
    try {
      const { WebGLAnime4KRenderer } = await import('/src/lib/player/webgl/engine.ts');
      const source = document.createElement('canvas');
      source.width = 96;
      source.height = 54;
      const sourceContext = source.getContext('2d');
      sourceContext.fillStyle = '#22cc88';
      sourceContext.fillRect(0, 0, source.width, source.height);

      const video = document.createElement('video');
      video.muted = true;
      video.autoplay = true;
      video.playsInline = true;
      const output = document.createElement('canvas');
      output.style.width = '192px';
      output.style.height = '108px';
      document.body.replaceChildren(video, output);
      video.srcObject = source.captureStream(30);
      let frame = 0;
      const timer = setInterval(() => {
        sourceContext.fillStyle = frame++ % 2 ? '#ff3366' : '#22cc88';
        sourceContext.fillRect(0, 0, source.width, source.height);
      }, 16);

      await Promise.race([
        video.play(),
        new Promise((_, reject) => setTimeout(() => reject(new Error('video play timeout')), 3000))
      ]);
      if (video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) {
        await Promise.race([
          new Promise((resolve) => video.addEventListener('loadeddata', resolve, { once: true })),
          new Promise((_, reject) => setTimeout(() => reject(new Error('video data timeout')), 3000))
        ]);
      }

      const report = { webgpu: !!navigator.gpu, brave: !!(navigator.brave && navigator.brave.isBrave), modes: {} };
      const modes = ${JSON.stringify(ONLY_MODE ? [ONLY_MODE] : ['anime4k-dog-x2', 'anime4k-lite', 'anime4k-a', 'anime4k-aa'])};
      for (const mode of modes) {
        let active = false;
        let failure = '';
        const started = await WebGLAnime4KRenderer.create({
          video, canvas: output, mode,
          onActiveChange: (value) => { active = value; },
          onFailure: (reason) => { failure = reason; }
        });
        if (!started.renderer) {
          report.modes[mode] = { error: started.reason ?? 'no renderer' };
          continue;
        }
        try {
          await new Promise((resolve, reject) => {
            const deadline = performance.now() + 12000;
            const poll = () => active ? resolve() : performance.now() > deadline
              ? reject(new Error('did not present: ' + failure))
              : setTimeout(poll, 50);
            poll();
          });
          report.modes[mode] = { active, size: [output.width, output.height] };
        } catch (error) {
          report.modes[mode] = { error: error.message };
        }
        started.renderer.stop();
      }
      clearInterval(timer);
      for (const track of video.srcObject.getTracks()) track.stop();
      return report;
    } catch (error) {
      return { error: error?.stack || String(error) };
    }
  })()`;
	const evaluated = await page.send('Runtime.evaluate', {
		expression,
		awaitPromise: true,
		returnByValue: true
	});
	console.log(JSON.stringify(evaluated.result.value ?? evaluated, null, 2));
} finally {
	page?.close();
	vite.kill('SIGTERM');
	brave.kill('SIGTERM');
	await sleep(250);
	if (vite.exitCode === null) vite.kill('SIGKILL');
	if (brave.exitCode === null) brave.kill('SIGKILL');
	rmSync(profile, { recursive: true, force: true });
}
