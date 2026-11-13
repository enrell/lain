<script lang="ts">
	/*
	 * Back to a page of cards (a library's grid, Search, the shelves, Home)
	 * puts the focus back on the card that was opened. The pages bring the
	 * scroll back themselves, but SvelteKit leaves the focus on the page's
	 * body, so the next Tab started over at the header and the arrows moved
	 * nothing.
	 *
	 * Leaving a page by one of its links remembers which (its address and
	 * which of the links to that address it was) under the page's address;
	 * Back or Forward to that address waits for the link to be drawn again,
	 * since the cards come after their data, and focuses it without
	 * scrolling. A key, a click or a focus of the viewer's own meanwhile
	 * wins. The focus ring shows only when the last input was a key, as a
	 * browser shows it.
	 */
	import { afterNavigate, beforeNavigate } from '$app/navigation';
	import { matchOpened, openedLink, type Opened } from './return-focus';

	const KEY = 'lain.focus.opened:';
	const WAIT_MS = 5000;
	// After the first focus, a list that redraws its cards may drop it on the body: put it back meanwhile.
	const SETTLE_MS = 1500;
	const placeOf = (url: URL): string => KEY + url.pathname + url.search;

	let keyboard = false;
	let clicked: { link: HTMLAnchorElement; at: number } | null = null;
	let stop: (() => void) | null = null;

	function onKeydown(): void {
		keyboard = true;
	}
	function onPointerdown(): void {
		keyboard = false;
	}
	function onClick(e: MouseEvent): void {
		const link = (e.target as Element | null)?.closest?.('a[href]');
		if (link instanceof HTMLAnchorElement) clicked = { link, at: Date.now() };
	}

	beforeNavigate((nav) => {
		stop?.();
		if (!nav.from || !nav.to || nav.type === 'popstate' || nav.willUnload) return;
		const main = document.getElementById('main');
		const recent = clicked && Date.now() - clicked.at < 1000 ? clicked.link : null;
		const opened = main ? openedLink(main, nav.to.url, [recent, document.activeElement]) : null;
		try {
			if (opened) sessionStorage.setItem(placeOf(nav.from.url), JSON.stringify(opened));
			else sessionStorage.removeItem(placeOf(nav.from.url));
		} catch {
			/* no storage: Back leaves the focus where SvelteKit puts it */
		}
	});

	afterNavigate((nav) => {
		if (nav.type !== 'popstate' || !nav.to) return;
		let opened: Opened | null = null;
		try {
			opened = JSON.parse(sessionStorage.getItem(placeOf(nav.to.url)) ?? 'null');
		} catch {
			return;
		}
		if (!opened || typeof opened.href !== 'string') return;
		const want = opened;
		const started = Date.now();
		let placed: HTMLElement | null = null;
		let placedAt = 0;
		const viewerMoved = (): void => cleanup();
		const settledFocus = (): boolean => {
			const active = document.activeElement;
			return active === null || active === document.body || active.id === 'main' || active === placed;
		};
		const attempt = (): void => {
			const now = Date.now();
			if (!placed && now - started > WAIT_MS) return cleanup();
			if (placed && now - placedAt > SETTLE_MS) return cleanup();
			// Something else took the focus (a field of the page's own, a dialog): leave it.
			if (!settledFocus()) return cleanup();
			if (placed?.isConnected && document.activeElement === placed) return;
			const main = document.getElementById('main');
			const link = main ? matchOpened(main, want) : null;
			if (!link) return;
			link.focus({ preventScroll: true, focusVisible: keyboard });
			// The page has put its scroll back by now (it does so as it draws,
			// before the next frame); one that does not leaves the card out of
			// sight, and the card is brought to the middle instead.
			const box = link.getBoundingClientRect();
			if (!placed && (box.bottom < 80 || box.top > innerHeight - 60 || box.right < 0 || box.left > innerWidth)) {
				link.scrollIntoView({ block: 'center', inline: 'nearest' });
			}
			placed = link;
			placedAt ||= now;
		};
		// A grid drawing hundreds of cards mutates the page many times a frame: look once a frame.
		let frame = 0;
		const observer = new MutationObserver(() => {
			frame ||= requestAnimationFrame(() => {
				frame = 0;
				attempt();
			});
		});
		const timer = setInterval(attempt, 250);
		function cleanup(): void {
			cancelAnimationFrame(frame);
			observer.disconnect();
			clearInterval(timer);
			window.removeEventListener('keydown', viewerMoved, true);
			window.removeEventListener('pointerdown', viewerMoved, true);
			if (stop === cleanup) stop = null;
		}
		stop?.();
		stop = cleanup;
		observer.observe(document.body, { childList: true, subtree: true });
		window.addEventListener('keydown', viewerMoved, true);
		window.addEventListener('pointerdown', viewerMoved, true);
		attempt();
	});
</script>

<svelte:window onkeydowncapture={onKeydown} onpointerdowncapture={onPointerdown} onclickcapture={onClick} />
