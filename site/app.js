// lain's site: the live demo's frames, tabs, copy buttons and the header's
// hairline. No dependencies.
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
		return { clear: () => tabs.forEach((t) => t.setAttribute('aria-selected', 'false')) };
	}

	// The live demo: the real client in an iframe, drawn at its own size and
	// scaled to its frame. It loads when asked for, not with the page.
	const frames = [...document.querySelectorAll('[data-live]')].map((box) => {
		const view = box.querySelector('.live-view, .phone-view');
		const frame = box.querySelector('iframe');
		const fit = () => view.style.setProperty('--scale', String(view.clientWidth / Number(frame.getAttribute('width'))));
		new ResizeObserver(fit).observe(view);
		fit();
		const show = (route) => {
			if (box.classList.contains('is-live')) {
				// Same origin: the demo routes by hash, so this is a navigation inside it.
				try {
					frame.contentWindow.location.hash = route;
					return;
				} catch {
					/* fall through and load it again */
				}
			}
			box.classList.add('is-live');
			frame.src = `${frame.dataset.src}#${route ?? '/'}`;
		};
		box.querySelector('[data-live-start]')?.addEventListener('click', () => show('/'));
		return { box, show, visible: () => box.offsetParent !== null };
	});
	// The frame a visitor can see: the browser frame on a wide screen, the phone on a narrow one.
	const stage = () => frames.find((f) => f.visible()) ?? frames[0];
	function go(route) {
		const f = stage();
		if (!f) return;
		f.show(route);
		f.box.scrollIntoView({ behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth', block: 'center' });
	}

	const demoTabs = document.querySelector('[data-demo-tabs]');
	if (demoTabs) tablist(demoTabs, (tab) => go(tab.dataset.route));
	document.addEventListener('click', (e) => {
		const b = e.target.closest('[data-demo-go]');
		if (b) go(b.dataset.demoGo);
	});

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
