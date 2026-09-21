/** Run against a loaded, seekable video in the disposable browser fixture. */
export async function verifyPlayerControls({ page, evalValue, pressKey, waitFor, assert }) {
	const video = `document.querySelector('video')`;
	const originalPath = await evalValue('location.pathname');
	const click = async (selector) => {
		const p = await evalValue(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return {x:r.x+r.width/2,y:r.y+r.height/2}; })()`);
		await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...p });
		for (const type of ['mousePressed', 'mouseReleased']) {
			await page.send('Input.dispatchMouseEvent', { type, ...p, button: 'left', clickCount: 1 });
		}
	};
	const atTen = async () => {
		await evalValue(`${video}.pause(); ${video}.currentTime = 10`);
		await waitFor(`${video}.paused && !${video}.seeking && Math.abs(Number(document.querySelector('[aria-label="Seek"]').getAttribute('aria-valuenow')) - 10) < 0.05`, 5000, 'paused at ten seconds');
	};
	/** The chrome fades while playback advances, so a pointer has to wake it
	 * before a click can land on a control. */
	const wake = async () => {
		const y = await evalValue(`document.querySelector('footer').getBoundingClientRect().top + 8`);
		await page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 100, y });
		await waitFor(`getComputedStyle(document.querySelector('footer')).opacity === '1'`, 5000, 'chrome revealed');
	};
	await atTen();
	await evalValue(`document.querySelector('[aria-label="Seek"]').focus()`);
	await pressKey('ArrowRight', 'ArrowRight', 39);
	await waitFor(`Math.abs(${video}.currentTime - 15) < 0.05`, 3000, 'focused seek advances five seconds');
	await pressKey('ArrowLeft', 'ArrowLeft', 37);
	await waitFor(`Math.abs(${video}.currentTime - 10) < 0.05`, 3000, 'focused seek rewinds five seconds');
	await pressKey(' ', 'Space', 32);
	await waitFor(`!${video}.paused`, 3000, 'Space on seek plays');
	await pressKey(' ', 'Space', 32);
	await waitFor(`${video}.paused`, 3000, 'Space on seek pauses');

	await click('button[aria-label="Mute"], button[aria-label="Unmute"]');
	await pressKey('k', 'KeyK', 75);
	await waitFor(`!${video}.paused`, 3000, 'K works after clicking mute');
	await pressKey('k', 'KeyK', 75);
	await waitFor(`${video}.paused`, 3000, 'K pauses with button focused');
	const muted = await evalValue(`${video}.muted`);
	await pressKey(' ', 'Space', 32);
	await waitFor(`${video}.muted !== ${muted}`, 3000, 'Space still activates focused mute button');
	assert(await evalValue(`${video}.paused`), 'native button Space also toggled playback');
	await pressKey('m', 'KeyM', 77);
	await waitFor(`${video}.muted === ${muted}`, 3000, 'M works on buttons');
	await atTen();
	await pressKey('l', 'KeyL', 76);
	await waitFor(`Math.abs(${video}.currentTime - 20) < 0.05`, 3000, 'L works on buttons');
	await pressKey('j', 'KeyJ', 74);
	await waitFor(`Math.abs(${video}.currentTime - 10) < 0.05`, 3000, 'J works on buttons');

	await click('button[aria-label="Playback settings"]');
	await waitFor(`document.activeElement?.tagName === 'SELECT'`, 3000, 'settings select receives focus');
	await pressKey('k', 'KeyK', 75);
	assert(await evalValue(`${video}.paused`), 'K hijacked a select');
	await pressKey('Escape', 'Escape', 27);
	await waitFor(`!document.querySelector('#player-settings')`, 3000, 'Escape dismisses settings');
	await evalValue('document.activeElement.blur()');
	await pressKey('Escape', 'Escape', 27);
	assert(await evalValue('location.pathname') === originalPath, 'windowed Escape navigated away');
	// The chrome stays while something asks for attention — a resume banner, a
	// failure, a wait — so the dismissal is measured once the banner's own
	// clock has cleared it.
	await waitFor(`!document.body.innerText.includes('Resuming from')`, 10000, 'resume banner cleared');
	await wake();
	await evalValue('document.activeElement.blur()');
	await pressKey('Escape', 'Escape', 27);
	await waitFor(`getComputedStyle(document.querySelector('footer')).opacity === '0'`, 3000, 'windowed Escape put the chrome away');

	await wake();
	await click('button[aria-label="Fullscreen"]');
	await waitFor(`document.fullscreenElement === document.querySelector('[aria-label^="Player —"]')`, 3000, 'player entered fullscreen');
	assert(await evalValue(`(() => { const r = document.fullscreenElement.getBoundingClientRect(); return Math.abs(r.width-innerWidth)<1 && Math.abs(r.height-innerHeight)<1; })()`), 'fullscreen player does not fill viewport');
	await pressKey('f', 'KeyF', 70);
	await waitFor('!document.fullscreenElement', 3000, 'F exits with fullscreen button focused');
	await pressKey('f', 'KeyF', 70);
	await waitFor('!!document.fullscreenElement', 3000, 'F reenters with button focused');
	await click('button[aria-label="Exit fullscreen"]');
	await waitFor('!document.fullscreenElement', 3000, 'fullscreen button exits');

	// Keep permission failure tests local to this browser; playback must survive.
	await evalValue('window.__originalFullscreen = Element.prototype.requestFullscreen');
	try {
		for (const replacement of [
			`function () { return Promise.reject(new DOMException('Test denial', 'NotAllowedError')); }`,
			'undefined'
		]) {
			await evalValue(`Element.prototype.requestFullscreen = ${replacement}`);
			await click('button[aria-label="Fullscreen"]');
			await waitFor(`!!document.querySelector('[data-fullscreen-notice]')`, 3000, 'fullscreen failure is visible');
			await pressKey('k', 'KeyK', 75);
			await waitFor(`!${video}.paused`, 3000, 'notice does not block playback');
			await pressKey('k', 'KeyK', 75);
			await click('button[aria-label="Dismiss fullscreen notice"]');
			await waitFor(`!document.querySelector('[data-fullscreen-notice]')`, 3000, 'notice dismisses');
			assert(await evalValue('location.pathname') === originalPath, 'fullscreen failure navigated away');
		}
	} finally {
		await evalValue('Element.prototype.requestFullscreen = window.__originalFullscreen; delete window.__originalFullscreen');
	}
	console.log('   player keyboard focus, five-second seek, Escape, fullscreen entry/exit and failure feedback passed');
}
