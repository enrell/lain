// lain's site: tabs, copy buttons and the header's hairline. No dependencies.
(() => {
	'use strict';

	// A tab list that follows the WAI-ARIA pattern: arrows move, Home and End jump.
	function tablist(root, onSelect) {
		const tabs = [...root.querySelectorAll('[role="tab"]')];
		const select = (tab, focus) => {
			for (const t of tabs) {
				const on = t === tab;
				t.setAttribute('aria-selected', String(on));
				t.tabIndex = on ? 0 : -1;
			}
			if (focus) tab.focus();
			onSelect(tab);
		};
		root.addEventListener('click', (e) => {
			const tab = e.target.closest('[role="tab"]');
			if (tab) select(tab, false);
		});
		root.addEventListener('keydown', (e) => {
			const i = tabs.indexOf(document.activeElement);
			if (i < 0) return;
			const to = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
			if (to === undefined) return;
			e.preventDefault();
			select(tabs[(to + tabs.length) % tabs.length], true);
		});
	}

	// The tour swaps one picture; each tab carries its file, alt text and caption.
	const tour = document.querySelector('[data-tour]');
	if (tour) {
		const img = tour.querySelector('[data-tour-img]');
		const cap = tour.querySelector('[data-tour-cap]');
		const panel = tour.querySelector('[role="tabpanel"]');
		const reduce = matchMedia('(prefers-reduced-motion: reduce)').matches;
		let wanted = img.getAttribute('src');
		tablist(tour.querySelector('[role="tablist"]'), (tab) => {
			const src = `assets/shots/${tab.dataset.shot}.webp`;
			panel.setAttribute('aria-labelledby', tab.id);
			cap.textContent = tab.dataset.cap;
			if (src === wanted) return;
			wanted = src;
			const next = new Image();
			const show = () => {
				if (wanted !== src) return;
				img.src = src;
				img.alt = tab.dataset.alt;
				img.classList.remove('is-swapping');
			};
			next.onload = next.onerror = show;
			if (!reduce) img.classList.add('is-swapping');
			next.src = src;
		});
		// Warm the other pictures once the page is idle, so a tab answers at once.
		const warm = () => {
			for (const t of tour.querySelectorAll('[role="tab"]')) new Image().src = `assets/shots/${t.dataset.shot}.webp`;
		};
		if ('requestIdleCallback' in window) requestIdleCallback(warm, { timeout: 4000 });
		else setTimeout(warm, 2500);
	}

	// Install: one panel per tab.
	const install = document.querySelector('[data-install]');
	if (install) {
		tablist(install.querySelector('[role="tablist"]'), (tab) => {
			for (const p of install.querySelectorAll('[role="tabpanel"]')) p.hidden = p.id !== tab.getAttribute('aria-controls');
		});
	}

	// Copy buttons say so, briefly.
	document.addEventListener('click', async (e) => {
		const b = e.target.closest('[data-copy]');
		if (!b) return;
		try {
			await navigator.clipboard.writeText(b.dataset.copy);
		} catch {
			return;
		}
		const label = b.textContent;
		b.textContent = 'Copied';
		b.classList.add('is-done');
		setTimeout(() => {
			b.textContent = label;
			b.classList.remove('is-done');
		}, 1400);
	});

	// The header gets its hairline once the page has moved.
	const nav = document.getElementById('nav');
	const stuck = () => nav.classList.toggle('is-stuck', scrollY > 8);
	addEventListener('scroll', stuck, { passive: true });
	stuck();
})();
