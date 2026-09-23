#!/usr/bin/env node
/*
 * Procedural WebGPU shader-engine probe.
 *
 * Starts the source Vite server, drives a real Chromium through CDP, creates
 * a same-origin synthetic video with canvas.captureStream(), imports the
 * production engine and proves ingest -> rgba16float -> WGSL pass -> canvas.
 * No Lain server, credentials or media files are involved.
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
const CHROMIUM = arg('chromium', 'chromium');

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
const profile = mkdtempSync(join(tmpdir(), 'lain-webgpu-'));
const vite = spawn('pnpm', ['dev', '--host', '127.0.0.1', '--port', String(vitePort)], {
	cwd: new URL('..', import.meta.url),
	stdio: 'ignore'
});
const chromium = spawn(
	CHROMIUM,
	[
		'--headless=new',
		'--no-first-run',
		'--no-default-browser-check',
		'--disable-dev-shm-usage',
		'--enable-unsafe-webgpu',
		'--enable-features=Vulkan,WebGPU',
		'--ignore-gpu-blocklist',
		'--use-angle=swiftshader',
		'--use-vulkan=swiftshader',
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
		waitHTTP(`http://127.0.0.1:${debugPort}/json/version`, 'Chromium DevTools')
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
      const { VideoRenderer } = await import('/src/lib/player/webgpu/engine.ts');
      const { ANIME4K_DOG_X2 } = await import('/src/lib/player/webgpu/packs/anime4k.ts');
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

      let active = false;
      let failure = '';
      const started = await VideoRenderer.create({
        video,
        canvas: output,
        passes: ANIME4K_DOG_X2,
        onActiveChange: (value) => active = value,
        onFailure: (reason) => failure = reason
      });
      if (!started.renderer) throw new Error(started.reason);
      await new Promise((resolve, reject) => {
        const deadline = performance.now() + 5000;
        const poll = () => active
          ? resolve()
          : performance.now() > deadline
            ? reject(new Error('renderer did not present: ' + failure))
            : setTimeout(poll, 50);
        poll();
      });
      const result = {
        active,
        source: [video.videoWidth, video.videoHeight],
        output: [output.width, output.height],
        webgpu: !!navigator.gpu
      };
      started.renderer.stop();
	  const modes = {};
	  for (const mode of ['anime4k-a', 'anime4k-aa', 'anime4k-lite']) {
	    active = false;
	    failure = '';
	    const next = await VideoRenderer.create({
	      video, canvas: output, computeMode: mode,
	      onActiveChange: (value) => active = value,
	      onFailure: (reason) => failure = reason
	    });
	    if (!next.renderer) throw new Error(mode + ': ' + next.reason);
	    await new Promise((resolve, reject) => {
	      const deadline = performance.now() + 12000;
	      const poll = () => active ? resolve() : performance.now() > deadline
	        ? reject(new Error(mode + ': renderer did not present: ' + failure))
	        : setTimeout(poll, 50);
	      poll();
	    });
	    modes[mode] = [output.width, output.height];
	    next.renderer.stop();
	  }
	  output.style.width = '384px';
	  output.style.height = '216px';
	  active = false;
	  const highScale = await VideoRenderer.create({
	    video, canvas: output, computeMode: 'anime4k-a',
	    onActiveChange: (value) => active = value,
	    onFailure: (reason) => failure = reason
	  });
	  if (!highScale.renderer) throw new Error(highScale.reason);
	  await new Promise((resolve, reject) => {
	    const deadline = performance.now() + 12000;
	    const poll = () => active ? resolve() : performance.now() > deadline
	      ? reject(new Error('4x renderer did not present: ' + failure))
	      : setTimeout(poll, 50);
	    poll();
	  });
	  result.highScale = [output.width, output.height];
	  highScale.renderer.stop();
	  result.autoDownscale = [];
	  for (const [width, height] of [[144, 81], [288, 162]]) {
	    output.style.width = width + 'px';
	    output.style.height = height + 'px';
	    active = false;
	    const scaled = await VideoRenderer.create({
	      video, canvas: output, computeMode: 'anime4k-a',
	      onActiveChange: (value) => active = value,
	      onFailure: (reason) => failure = reason
	    });
	    if (!scaled.renderer) throw new Error(scaled.reason);
	    await new Promise((resolve, reject) => {
	      const deadline = performance.now() + 12000;
	      const poll = () => active ? resolve() : performance.now() > deadline
	        ? reject(new Error('auto downscale did not present: ' + failure))
	        : setTimeout(poll, 50);
	      poll();
	    });
	    result.autoDownscale.push([output.width, output.height]);
	    scaled.renderer.stop();
	  }
	  result.modes = modes;
      clearInterval(timer);
      for (const track of video.srcObject.getTracks()) track.stop();
      return result;
    } catch (error) {
      return { error: error?.stack || String(error) };
    }
  })()`;
	const evaluated = await page.send('Runtime.evaluate', {
		expression,
		awaitPromise: true,
		returnByValue: true
	});
	const result = evaluated.result.value;
	if (result?.error) throw new Error(result.error);
	if (!result?.active || !result.webgpu) throw new Error(`renderer inactive: ${JSON.stringify(result)}`);
	if (result.source.join('x') !== '96x54' || result.output.join('x') !== '192x108') {
		throw new Error(`wrong graph dimensions: ${JSON.stringify(result)}`);
	}
	if (result.modes['anime4k-lite'].join('x') !== '96x54' ||
		result.modes['anime4k-a'].join('x') !== '192x108' ||
		result.modes['anime4k-aa'].join('x') !== '192x108' ||
		result.highScale.join('x') !== '384x216' ||
		result.autoDownscale[0].join('x') !== '144x81' ||
		result.autoDownscale[1].join('x') !== '288x162') {
		throw new Error(`wrong Anime4K dimensions: ${JSON.stringify(result)}`);
	}
	console.log(`WebGPU probe passed: ${result.source.join('x')} -> ${result.output.join('x')}`);
} finally {
	page?.close();
	vite.kill('SIGTERM');
	chromium.kill('SIGTERM');
	await sleep(250);
	if (vite.exitCode === null) vite.kill('SIGKILL');
	if (chromium.exitCode === null) chromium.kill('SIGKILL');
	rmSync(profile, { recursive: true, force: true });
}
