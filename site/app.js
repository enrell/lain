// lain's site. One frame loop drives everything that moves: a smoothed scroll
// position, a shader behind the page, and scenes whose state is a function of
// how far the scroll is through them. No dependencies.
(() => {
	'use strict';

	const calm = matchMedia('(prefers-reduced-motion: reduce)').matches;
	const fine = matchMedia('(hover: hover) and (pointer: fine)').matches;
	const root = document.documentElement;
	const NAV = 60;

	const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));
	const lerp = (a, b, t) => a + (b - a) * t;
	const ramp = (a, b, v) => {
		const t = clamp((v - a) / (b - a));
		return t * t * (3 - 2 * t);
	};
	const out3 = (t) => 1 - (1 - clamp(t)) ** 3;
	const top = (el) => el.getBoundingClientRect().top + scrollY;

	// ---- scroll: the wheel moves a target, the page eases to it -------------
	const scroll = { y: scrollY, target: scrollY, v: 0, smooth: !calm && fine };
	if (scroll.smooth) root.classList.add('smooth');
	const maxY = () => root.scrollHeight - innerHeight;
	// True when something under the pointer can itself scroll that way.
	const scrollsInside = (el, dy) => {
		for (; el && el !== document.body && el instanceof Element; el = el.parentElement) {
			if (el.scrollHeight <= el.clientHeight + 1) continue;
			const o = getComputedStyle(el).overflowY;
			if (o !== 'auto' && o !== 'scroll') continue;
			if (dy < 0 ? el.scrollTop > 0 : el.scrollTop + el.clientHeight < el.scrollHeight - 1) return true;
		}
		return false;
	};
	if (scroll.smooth) {
		addEventListener(
			'wheel',
			(e) => {
				if (e.ctrlKey || e.defaultPrevented || document.querySelector('dialog[open]')) return;
				if (Math.abs(e.deltaX) > Math.abs(e.deltaY) || scrollsInside(e.target, e.deltaY)) return;
				e.preventDefault();
				const unit = e.deltaMode === 1 ? 100 / 6 : e.deltaMode === 2 ? innerHeight : 1;
				scroll.target = clamp(scroll.target + e.deltaY * unit, 0, maxY());
			},
			{ passive: false }
		);
		// A scroll this loop did not make (keys, the scrollbar, a find-in-page) is followed, not fought.
		addEventListener(
			'scroll',
			() => {
				if (Math.abs(scrollY - scroll.y) > 3) scroll.y = scroll.target = scrollY;
			},
			{ passive: true }
		);
	}
	function scrollToY(y) {
		y = clamp(y, 0, maxY());
		if (scroll.smooth) scroll.target = y;
		else scrollTo({ top: y, behavior: calm ? 'auto' : 'smooth' });
	}
	const scrollToEl = (el, gap = NAV + 16) => scrollToY(top(el) - gap);
	// A jump also moves the keyboard: Tab goes on from the heading it landed on.
	const jumpTo = (section) => {
		const head = section.querySelector('.chapter-head') ?? section;
		scrollToEl(head, NAV + 40);
		const h = head.querySelector('h2') ?? head;
		if (!h.hasAttribute('tabindex')) h.setAttribute('tabindex', '-1');
		h.focus({ preventScroll: true });
	};

	// ---- the pointer, eased ------------------------------------------------
	const mouse = { x: 0, y: 0, tx: 0, ty: 0 };
	if (fine && !calm) {
		addEventListener(
			'pointermove',
			(e) => {
				mouse.tx = e.clientX / innerWidth - 0.5;
				mouse.ty = e.clientY / innerHeight - 0.5;
			},
			{ passive: true }
		);
	}

	// ---- scenes: each has a stretch of the page and a state for every point of it
	const scenes = [];
	// range() answers [start, end] in page pixels; update(p) gets 0..1 across it.
	const scene = (range, update, ease = 0.16) => scenes.push({ range, update, ease, a: 0, b: 1, p: -1 });
	const measure = () => {
		for (const s of scenes) [s.a, s.b] = s.range();
	};

	// A tab list that follows the WAI-ARIA pattern: arrows move, Home and End jump.
	function tablist(el, onSelect) {
		const tabs = [...el.querySelectorAll('[role="tab"]')];
		const select = (tab, focus) => {
			for (const t of tabs) {
				const on = t === tab;
				t.setAttribute('aria-selected', String(on));
				t.tabIndex = on ? 0 : -1;
			}
			if (focus) tab.focus();
			onSelect(tab);
		};
		el.addEventListener('click', (e) => {
			const tab = e.target.closest('[role="tab"]');
			if (tab) select(tab, false);
		});
		el.addEventListener('keydown', (e) => {
			const i = tabs.indexOf(document.activeElement);
			if (i < 0) return;
			const to = { ArrowRight: i + 1, ArrowLeft: i - 1, Home: 0, End: tabs.length - 1 }[e.key];
			if (to === undefined) return;
			e.preventDefault();
			select(tabs[(to + tabs.length) % tabs.length], true);
		});
	}

	// A wheel turned over the client that the page should answer.
	const wheelUnit = (e) => (e.deltaMode === 1 ? 100 / 6 : e.deltaMode === 2 ? innerHeight : 1);
	const pageBy = (dy) => {
		if (scroll.smooth) scroll.target = clamp(scroll.target + dy, 0, maxY());
		else scrollBy(0, dy);
	};

	// A phone-sized client under a mouse is treated like a phone under a finger: the
	// pointer is a fingertip, a drag scrolls instead of lifting a picture out of the
	// page, and a drag that moved is not a tap.
	const fingertip =
		"url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='30' height='30'%3E%3Ccircle cx='15' cy='15' r='12' fill='rgba(250,252,251,.22)' stroke='rgba(250,252,251,.7)' stroke-width='1.5'/%3E%3C/svg%3E\") 15 15, pointer";
	function touchify(win) {
		if (!fine) return;
		const doc = win.document;
		const style = doc.createElement('style');
		style.textContent =
			`*, *::before, *::after { cursor: ${fingertip} !important; }` +
			'img, a, svg { -webkit-user-drag: none; user-drag: none; } html { user-select: none; -webkit-user-select: none; }' +
			'input, textarea { user-select: text; -webkit-user-select: text; }';
		doc.head.append(style);
		doc.addEventListener('dragstart', (e) => e.preventDefault(), true);
		// The scroller a drag should move along one axis, starting from what was pressed.
		const scroller = (node, axis) => {
			for (; node && node.nodeType === 1; node = node.parentElement) {
				const page = node === doc.documentElement || node === doc.body;
				const room = axis === 'x' ? node.scrollWidth - node.clientWidth : node.scrollHeight - node.clientHeight;
				if (room <= 1) continue;
				const o = win.getComputedStyle(node)[axis === 'x' ? 'overflowX' : 'overflowY'];
				if (page) return doc.scrollingElement;
				if (o === 'auto' || o === 'scroll') return node;
			}
			return null;
		};
		let drag = null;
		let moved = false;
		let glide = 0;
		doc.addEventListener(
			'pointerdown',
			(e) => {
				if (e.pointerType !== 'mouse' || e.button !== 0 || e.target.closest('input, textarea, select, [contenteditable]')) return;
				cancelAnimationFrame(glide);
				moved = false;
				drag = { x: e.clientX, y: e.clientY, sx: scroller(e.target, 'x'), sy: scroller(e.target, 'y'), vx: 0, vy: 0, far: 0 };
			},
			true
		);
		doc.addEventListener(
			'pointermove',
			(e) => {
				if (!drag) return;
				if (!(e.buttons & 1)) return (drag = null);
				const dx = e.clientX - drag.x;
				const dy = e.clientY - drag.y;
				drag.x = e.clientX;
				drag.y = e.clientY;
				drag.far += Math.abs(dx) + Math.abs(dy);
				if (drag.far < 6) return;
				moved = true;
				if (drag.sx) drag.sx.scrollLeft -= dx;
				if (drag.sy) drag.sy.scrollTop -= dy;
				drag.vx = dx;
				drag.vy = dy;
			},
			true
		);
		const release = () => {
			const d = drag;
			drag = null;
			if (!d || !moved) return;
			// Let go while moving and it keeps going a little, as a flick does.
			const coast = () => {
				d.vx *= 0.94;
				d.vy *= 0.94;
				if (Math.abs(d.vx) + Math.abs(d.vy) < 0.4) return;
				if (d.sx) d.sx.scrollLeft -= d.vx;
				if (d.sy) d.sy.scrollTop -= d.vy;
				glide = win.requestAnimationFrame(coast);
			};
			coast();
		};
		doc.addEventListener('pointerup', release, true);
		doc.addEventListener('pointercancel', release, true);
		// What follows a drag is not a tap.
		doc.addEventListener(
			'click',
			(e) => {
				if (!moved) return;
				moved = false;
				e.preventDefault();
				e.stopPropagation();
			},
			true
		);
	}

	// A client in the page takes the keyboard only while someone is using it: it may
	// not pull focus to itself on load or on a route change, and Esc inside it gives
	// the keyboard back to the page (unless the client used Esc for itself).
	function guard(frame, win, using, leave) {
		win.addEventListener(
			'focusin',
			() => {
				if (using()) return;
				try {
					win.document.activeElement?.blur();
				} catch {
					/* gone */
				}
				if (document.activeElement === frame) frame.blur();
			},
			true
		);
		win.addEventListener('keydown', (e) => {
			if (e.key !== 'Escape' || e.defaultPrevented || !using()) return;
			if (win.document.querySelector('dialog[open], [role="dialog"], [role="menu"]')) return;
			leave();
		});
	}

	// ---- hero: screens of the real client in depth, a camera the scroll flies
	// through them, and whichever is clicked comes to the centre to be used.
	const hero = document.getElementById('top');
	const rail = document.querySelector('[data-rail]');
	if (rail) rail.inert = true;
	// A video started inside a client stops when the client is left: the client reloads
	// on its route, back to the poster.
	const hush = (frame) => {
		try {
			const doc = frame.contentDocument;
			if (doc?.querySelector('iframe[src*="youtube"]')) frame.contentWindow.location.reload();
		} catch {
			/* not ours to touch */
		}
	};
	const stage = document.querySelector('[data-stage]');
	const planesEl = document.querySelector('[data-planes]');
	const words = document.querySelector('[data-box]');
	const hint = document.querySelector('.scroll-hint');
	const unfocusBtn = document.querySelector('[data-unfocus]');
	const planes = [...document.querySelectorAll('.plane')].map((el, i) => {
		const [x, y, z, rx, ry, rz, w] = el.dataset.at.split(',').map(Number);
		const phone = el.dataset.kind === 'phone';
		const frameEl = el.querySelector('iframe');
		const nw = Number(frameEl.getAttribute('width'));
		const nh = Number(frameEl.getAttribute('height'));
		// On an upright phone the two phone screens stand side by side under the words.
		const [nx, ny, nrz] = (el.dataset.narrow ?? `${x},${y},${rz}`).split(',').map(Number);
		return { el, x, y, z, rx, ry, rz, w, phone, nw, nh, nx, ny, nrz, depth: Number(el.dataset.depth), i, f: 0, h: 0, h2: 0, base: 0, done: false, over: false, frame: el.querySelector('iframe'), route: el.dataset.route, asked: false };
	});
	// A plane is laid out at the width it is shown at and the client inside it is
	// zoomed, not scaled: its text is drawn at that size and stays sharp. The
	// transform only scales while a plane is on its way somewhere.
	const canZoom = 'zoom' in root.style;
	const setBase = (pl, width) => {
		const w = Math.round(width);
		if (Math.abs(w - pl.base) < 2) return;
		pl.base = w;
		const u = w / pl.nw;
		pl.high = Math.round((w * pl.nh) / pl.nw);
		pl.el.style.width = `${w}px`;
		pl.el.style.height = `${pl.high}px`;
		pl.el.style.setProperty('--u', u.toFixed(4));
		// A whole number of pixels, so the client inside starts on one.
		pl.bezel = pl.phone ? Math.max(2, Math.round(7 * u)) : 1;
		pl.el.style.setProperty('--bezel', `${pl.bezel}px`);
		if (canZoom) pl.frame.style.zoom = String(u);
		else pl.frame.style.transform = `scale(${u})`;
	};
	let born = performance.now();
	let heroP = 0;
	let focused = null;
	let away = 0;
	const narrowNow = () => innerWidth < 760;

	function focusPlane(pl) {
		if (focused === pl) return;
		const was = focused;
		was?.el.classList.remove('is-focused');
		focused = pl;
		planesEl.classList.toggle('has-focus', Boolean(pl));
		if (unfocusBtn) unfocusBtn.hidden = !pl;
		for (const p of planes) p.frame.inert = p !== pl;
		if (was) hush(was.frame);
		if (!pl) {
			// The keyboard comes back to the screen it left from.
			if (was && (document.activeElement === was.frame || document.activeElement === document.body)) was.el.focus({ preventScroll: true });
			return;
		}
		pl.el.classList.add('is-focused');
		loadPlane(pl);
		// The keys belong to the client once it is in the centre.
		pl.frame.focus({ preventScroll: true });
	}
	const unfocus = () => focusPlane(null);
	function loadPlane(pl) {
		if (pl.asked) return;
		pl.asked = true;
		pl.frame.addEventListener('load', () => {
			let win;
			try {
				win = pl.frame.contentWindow;
				// Anything but the client (a missing build, an error page) leaves the picture up.
				if (!/lain/i.test(win.document.title) || !win.document.querySelector('script')) throw new Error('not the client');
			} catch {
				pl.done = true;
				return;
			}
			if (pl.phone) touchify(win);
			guard(pl.frame, win, () => focused === pl, unfocus);
			// In the centre the client scrolls first and the page takes over at its edges.
			win.addEventListener(
				'wheel',
				(e) => {
					if (e.ctrlKey) return;
					const doc = win.document;
					for (let node = e.target; node && node.nodeType === 1; node = node.parentElement) {
						const page = node === doc.documentElement || node === doc.body;
						if (node.scrollHeight <= node.clientHeight + 1) continue;
						const o = win.getComputedStyle(node).overflowY;
						if (!page && o !== 'auto' && o !== 'scroll') continue;
						const box = page ? doc.scrollingElement : node;
						if (e.deltaY < 0 ? box.scrollTop > 0 : box.scrollTop + box.clientHeight < box.scrollHeight - 1) return;
					}
					e.preventDefault();
					pageBy(e.deltaY * wheelUnit(e));
				},
				{ passive: false, capture: true }
			);
			// Its first screen has drawn once the client's own pictures have settled.
			const settled = () =>
				win.document.images.length > 0 &&
				[...win.document.images].every((img) => img.complete || img.getBoundingClientRect().top > win.innerHeight);
			// The client has drawn when it has put words on its page. One that never does
			// (a worker that will not take control, say) leaves its picture up instead.
			const drawn = () => (win.document.body?.innerText ?? '').trim().length > 20;
			const since = performance.now();
			const poll = setInterval(() => {
				const waited = performance.now() - since;
				if (!drawn()) {
					if (waited < 6000) return;
					clearInterval(poll);
					pl.done = true;
					return;
				}
				if (waited < 250 || (!settled() && waited < 1400)) return;
				clearInterval(poll);
				pl.el.classList.add('is-live');
				pl.done = true;
			}, 80);
		});
		pl.frame.src = `${pl.frame.dataset.src}#${pl.route}`;
	}
	for (const pl of planes) {
		pl.el.addEventListener('pointerenter', () => (pl.over = true));
		pl.el.addEventListener('pointerleave', () => (pl.over = false));
		pl.el.addEventListener('click', () => focusPlane(pl));
		pl.el.addEventListener('keydown', (e) => {
			if (e.target !== pl.el || (e.key !== 'Enter' && e.key !== ' ')) return;
			e.preventDefault();
			focusPlane(pl);
		});
	}
	// A click on the empty stage, the button or Esc puts the screen back.
	stage?.addEventListener('click', (e) => {
		if (focused && !e.target.closest('.plane, [data-hero-open], a, button')) unfocus();
	});
	unfocusBtn?.addEventListener('click', unfocus);
	addEventListener('keydown', (e) => {
		if (e.key === 'Escape' && focused) unfocus();
	});
	// From further down, the page rides up first and the screen comes forward on arrival.
	let pending = null;
	const openHero = () => {
		const pl = planes.find((p) => (narrowNow() ? p.phone : !p.phone)) ?? planes[0];
		if (heroP < 0.05) return focusPlane(pl);
		pending = pl;
		scrollToY(0);
	};
	for (const b of document.querySelectorAll('[data-hero-open]')) b.addEventListener('click', openHero);

	const drawPlanes = (now) => {
		const narrow = narrowNow();
		const p = calm ? 0 : heroP;
		if (pending && heroP < 0.05) {
			focusPlane(pending);
			pending = null;
		}
		// Scrolling on takes the camera with it: the screen goes back to its place.
		if (focused && heroP > 0.1) unfocus();
		away = lerp(away, focused ? 1 : 0, 0.09);
		// In the centre: flat, facing the reader, as large as the window allows.
		const fit = (pl) => (pl.phone ? Math.min(innerWidth * 0.86, (innerHeight - 150) * 0.4875) : Math.min(innerWidth * 0.66, (innerHeight - 170) * 1.6));
		// The rest wait in the margins either side of it: two on the left, one on the right.
		const lead = focused ?? planes.find((pl) => pl.f > 0.01);
		if (lead) {
			const gutter = (innerWidth - fit(lead)) / 2;
			const slots = narrow ? [[-1, -0.36], [1, -0.36], [1, 0.36]] : [[-1, -0.17], [-1, 0.21], [1, 0.02]];
			planes
				.filter((pl) => pl !== lead && !(narrow && !pl.phone))
				.forEach((pl, k) => {
					const [side, at] = slots[k] ?? slots[0];
					const w = Math.min(gutter * 0.74, pl.phone ? innerHeight * 0.3 * 0.4875 : 9e9);
					pl.dock = { x: side * (innerWidth / 2 - gutter / 2), y: innerHeight * at, w, ry: side * -16 };
				});
		}
		for (const pl of planes) {
			// A 1440 px client is no use on an upright phone: only the phone screens stand there.
			const hidden = narrow && !pl.phone;
			pl.el.hidden = hidden;
			if (hidden) continue;
			pl.f = lerp(pl.f, pl === focused ? 1 : 0, 0.1);
			pl.h = lerp(pl.h, pl.over && !focused ? 1 : 0, 0.14);
			// Hovered while another is in the centre: it lights up, to say it can take its place.
			pl.h2 = lerp(pl.h2, pl.over && focused && pl !== focused ? 1 : 0, 0.14);
			const e = pl.f * pl.f * (3 - 2 * pl.f);
			const others = pl.dock ? away * (1 - e) : 0;
			// The entrance: each plane arrives from far away, one after the other.
			const t = calm ? 1 : out3((now - born - 150 - pl.i * 130) / 1700);
			const spread = 1 + p * (0.5 + pl.depth * 0.5);
			const dock = pl.dock ?? { x: 0, y: 0, w: 0, ry: 0 };
			// Where it stands by itself, then eased towards its place in the margin.
			const w0 = (pl.w * (narrow ? 2.7 : 1) * innerWidth) / 100;
			const x0 = (((narrow ? pl.nx : pl.x) * innerWidth) / 100) * spread + mouse.x * -46 * pl.depth;
			const y0 = (((narrow ? pl.ny : pl.y) * innerHeight) / 100) * spread + mouse.y * -30 * pl.depth;
			const z0 = pl.z + p * (900 + pl.depth * 900) - (1 - t) * 1700 + pl.h * 90;
			// Tilted, a screen is drawn at the client's full size and brought down by the
			// transform: measured, that keeps its text cleanest. In the centre it is drawn
			// at exactly the size it is shown, on whole pixels.
			const landed = pl === focused && pl.f > 0.992;
			if (landed) setBase(pl, fit(pl));
			else if ((pl.f < 0.01 && others < 0.01) || !pl.base) setBase(pl, pl.nw);
			const w = lerp(w0, dock.w, others);
			const x = lerp(x0, dock.x, others);
			const y = lerp(y0, dock.y, others);
			const z = lerp(z0, -60 + pl.h2 * 110, others);
			const ry = lerp(pl.ry + mouse.x * 7, dock.ry, others);
			const loose = 1 - e;
			pl.el.style.opacity = (t * (1 - ramp(520, 980, z) * loose) * lerp(0.9, 1, clamp(pl.depth)) * (1 - others * (0.42 - pl.h2 * 0.42))).toFixed(3);
			pl.el.style.zIndex = pl === focused ? '3' : '';
			if (landed) {
				// No rotation, no scale, and a corner that sits on a pixel: nothing is resampled.
				const box = stage.getBoundingClientRect();
				const wide = pl.base + pl.bezel * 2;
				const tall = pl.high + pl.bezel * 2;
				const left = Math.round(box.width / 2 - wide / 2) - box.width / 2;
				const up = Math.round(box.height / 2 + innerHeight * 0.035 - tall / 2) - box.height / 2;
				pl.el.style.transform = `translate3d(${left}px, ${up}px, 0)`;
				continue;
			}
			pl.el.style.transform =
				`translate(-50%, -50%) translate3d(${lerp(x, 0, e).toFixed(1)}px, ${lerp(y, innerHeight * 0.035, e).toFixed(1)}px, ${lerp(z, 0, e).toFixed(1)}px) ` +
				`rotateX(${((pl.rx + mouse.y * -5) * loose * (1 - others)).toFixed(2)}deg) rotateY(${(ry * loose * (1 - pl.h * 0.35)).toFixed(2)}deg) ` +
				`rotateZ(${((narrow ? pl.nrz : pl.rz) * loose * (1 - others)).toFixed(2)}deg) scale(${(lerp(w, fit(pl), e) / pl.base).toFixed(4)})`;
		}
		if (words) {
			const gone = Math.max(ramp(0.3, 0.62, p), away);
			words.style.transform = `translate3d(${mouse.x * 10}px, ${mouse.y * 8}px, 0) scale(${1 + p * 0.22 - away * 0.06})`;
			words.style.opacity = String(1 - gone);
			words.style.visibility = gone > 0.98 ? 'hidden' : '';
		}
		if (hint) hint.style.opacity = String((1 - ramp(0, 0.12, p)) * (1 - away));
	};
	if (hero) {
		scene(
			() => [0, hero.offsetHeight - innerHeight],
			(p) => {
				heroP = p;
				document.body.classList.toggle('past-hero', p > 0.7);
				// Invisible, the rail is out of the tab order too.
				if (rail) rail.inert = p <= 0.7;
			}
		);
	}

	// ---- the manifesto: words light up as the scroll passes them -------------
	const manifesto = document.querySelector('[data-manifesto] p');
	if (manifesto) {
		const words = [];
		const split = (node) => {
			for (const child of [...node.childNodes]) {
				if (child.nodeType === 3) {
					const frag = document.createDocumentFragment();
					for (const part of child.textContent.split(/(\s+)/)) {
						if (!part.trim()) {
							frag.append(part);
							continue;
						}
						const s = document.createElement('span');
						s.className = 'w';
						s.textContent = part;
						words.push(s);
						frag.append(s);
					}
					child.replaceWith(frag);
				} else if (child.dataset?.linuxSizeShort !== undefined) {
					child.classList.add('w');
					words.push(child);
				} else split(child);
			}
		};
		split(manifesto);
		if (!calm) {
			scene(
				() => [top(manifesto) - innerHeight * 0.82, top(manifesto) + manifesto.offsetHeight - innerHeight * 0.42],
				(p) => {
					const at = p * (words.length + 4);
					words.forEach((w, i) => (w.style.opacity = lerp(0.14, 1, clamp((at - i) / 4)).toFixed(3)));
				}
			);
		}
	}

	// ---- chapter titles: words rise out of their line; the numeral drifts behind
	for (const h of document.querySelectorAll('.chapter-head h2')) {
		let i = 0;
		const wrap = (node) => {
			for (const child of [...node.childNodes]) {
				if (child.nodeType !== 3) {
					wrap(child);
					continue;
				}
				const frag = document.createDocumentFragment();
				for (const part of child.textContent.split(/(\s+)/)) {
					if (!part.trim()) {
						frag.append(part);
						continue;
					}
					const outer = document.createElement('span');
					outer.className = 'w';
					const inner = document.createElement('span');
					inner.style.setProperty('--i', String(i++));
					inner.textContent = part;
					outer.append(inner);
					frag.append(outer);
				}
				child.replaceWith(frag);
			}
		};
		wrap(h);
	}
	for (const n of document.querySelectorAll('.numeral')) {
		if (calm) break;
		const head = n.parentElement;
		scene(
			() => [top(head) - innerHeight, top(head) + head.offsetHeight],
			(p) => (n.style.transform = `translate3d(0, ${((0.5 - p) * 260).toFixed(1)}px, 0)`),
			0.1
		);
	}

	// ---- seen once: titles, rows, keys, figures ------------------------------
	const seen = new IntersectionObserver(
		(entries) => {
			for (const e of entries) {
				if (!e.isIntersecting) continue;
				e.target.classList.add('is-in');
				e.target.dispatchEvent(new CustomEvent('seen'));
				seen.unobserve(e.target);
			}
		},
		{ rootMargin: '0px 0px -14% 0px' }
	);
	for (const el of document.querySelectorAll('.chapter-head, .rows, .caps, .diff, .rise, .stats, [data-term]')) seen.observe(el);
	for (const list of document.querySelectorAll('.rows, .caps')) [...list.children].forEach((c, i) => c.style.setProperty('--i', String(i)));
	for (const d of document.querySelectorAll('.diff')) {
		d.querySelectorAll('.del').forEach((l, i) => l.style.setProperty('--i', String(i)));
		d.querySelectorAll('.add').forEach((l, i) => l.style.setProperty('--i', String(i)));
	}

	// ---- the showcase: a pinned screen holding the real client. It loads with the
	// page; each step of the scroll takes it to another route, and whoever stops
	// can use it. The pictures stand in until it has loaded, and where it cannot.
	const reels = [...document.querySelectorAll('[data-reel]')].map((el) => {
		const liveBox = el.querySelector('[data-live]');
		const view = liveBox.querySelector('.live-view, .phone-view');
		const frame = liveBox.querySelector('iframe');
		const holder = el.querySelector('.reel-frame');
		const shots = [...view.querySelectorAll('.shot')];
		const caps = [...el.querySelectorAll('.reel-cap')];
		const routes = [...el.querySelectorAll('.reel-step')].map((s) => s.dataset.route ?? '');
		const dots = [...el.querySelectorAll('.reel-dots button')];
		const status = el.querySelector('[data-status]');
		const n = shots.length;
		const phone = el.classList.contains('reel-phone');
		const fit = () => {
			if (!view.clientWidth) return;
			const u = view.clientWidth / Number(frame.getAttribute('width'));
			if ('zoom' in frame.style) frame.style.zoom = String(u);
			else frame.style.transform = `scale(${u})`;
		};
		new ResizeObserver(fit).observe(view);

		let at = -1;
		let route = routes[0] || '/';
		let asked = false;
		let hover = 0;
		const live = () => el.classList.contains('is-live');
		const say = (text) => status && (status.textContent = text);
		// Driving: a click inside the client hands it the wheel too, until the next step.
		const driving = () => el.classList.contains('is-driving');
		const idle = () => (el.classList.contains('is-still') ? 'A picture of the Android app' : fine ? 'Live. Click and use it' : 'Live. Tap to use it');
		const engage = (on) => {
			// Under the pointer the client can be clicked straight away; otherwise it is out of the tab order.
			frame.inert = !on && !(fine && hover);
			if (driving() === on) return;
			el.classList.toggle('is-driving', on);
			if (!on) hush(frame);
			if (on) frame.focus({ preventScroll: true });
			else if (document.activeElement === frame) holder.focus({ preventScroll: true });
			say(on ? (fine ? 'You are driving. Esc or scroll past its edge to move on' : 'You are driving. Swipe outside it to move on') : idle());
		};
		const insideScrolls = (target, dy) => {
			const doc = frame.contentDocument;
			for (let node = target; node && node.nodeType === 1; node = node.parentElement) {
				const page = node === doc.documentElement || node === doc.body;
				if (node.scrollHeight <= node.clientHeight + 1) continue;
				const o = frame.contentWindow.getComputedStyle(node).overflowY;
				if (!page && o !== 'auto' && o !== 'scroll') continue;
				const box = page ? doc.scrollingElement : node;
				if (dy < 0 ? box.scrollTop > 0 : box.scrollTop + box.clientHeight < box.scrollHeight - 1) return true;
			}
			return false;
		};
		const wire = () => {
			let win;
			try {
				win = frame.contentWindow;
				// Anything but the client (a missing build, an error page) leaves the pictures up.
				if (!/lain/i.test(win.document.title) || !win.document.querySelector('script')) return;
			} catch {
				return;
			}
			if (phone) touchify(win);
			// The wheel belongs to the page until someone is driving; then the client
			// scrolls first and the page takes over at its edges.
			win.addEventListener(
				'wheel',
				(e) => {
					if (e.ctrlKey) return;
					if (el.classList.contains('is-driving') && insideScrolls(e.target, e.deltaY)) return;
					e.preventDefault();
					pageBy(e.deltaY * wheelUnit(e));
				},
				{ passive: false, capture: true }
			);
			win.addEventListener('pointerdown', () => engage(true), true);
			guard(frame, win, driving, () => engage(false));
			setTimeout(() => {
				el.classList.add('is-live');
				fit();
				engage(false);
				say(idle());
				if (routes[Math.max(0, at)] && win.location.hash.slice(1) !== route) win.location.hash = route;
			}, 500);
		};
		frame.addEventListener('load', wire);
		const load = () => {
			if (asked) return;
			asked = true;
			say('Loading the client');
			frame.src = `${frame.dataset.src}#${route}`;
		};
		const show = (to) => {
			route = to || route;
			if (!live()) return load();
			// Same origin: the demo routes by hash, so this is a navigation inside it.
			try {
				if (frame.contentWindow.location.hash.slice(1) !== route) frame.contentWindow.location.hash = route;
			} catch {
				frame.src = `${frame.dataset.src}#${route}`;
			}
		};
		// On a touch screen the client is behind glass until it is tapped, so a swipe
		// over it still scrolls the page.
		view.addEventListener('click', () => {
			if (live() && !el.classList.contains('is-still')) engage(true);
		});
		holder.addEventListener('pointerenter', () => {
			hover = 1;
			if (live()) frame.inert = false;
		});
		holder.addEventListener('pointerleave', () => {
			hover = 0;
			if (!driving()) frame.inert = true;
		});
		// By keyboard: the frame is a control; Enter hands it the keys.
		holder.tabIndex = 0;
		holder.setAttribute('role', 'button');
		holder.setAttribute('aria-label', phone ? 'Use the live client at phone size' : 'Use the live client');
		holder.addEventListener('keydown', (e) => {
			if (e.target !== holder || (e.key !== 'Enter' && e.key !== ' ')) return;
			e.preventDefault();
			if (live() && !el.classList.contains('is-still')) engage(true);
		});

		const range = () => [top(el) - NAV, top(el) + el.offsetHeight - innerHeight];
		const settle = (i) => {
			if (i === at) return;
			const first = at < 0;
			at = i;
			dots.forEach((d, k) => (k === i ? d.setAttribute('aria-current', 'true') : d.removeAttribute('aria-current')));
			caps.forEach((c, k) => c.classList.toggle('is-on', k === i));
			for (const s of [shots[i + 1], shots[i + 2]]) s?.setAttribute('loading', 'eager');
			if (!first) engage(false);
			if (routes[i]) {
				route = routes[i];
				if (live()) show(route);
			}
		};
		scene(range, (p) => {
			const f = calm ? Math.round(p * (n - 1)) : p * (n - 1);
			const i = clamp(Math.round(f), 0, n - 1);
			settle(i);
			// A screen holds for most of its step, then hands over in the middle third.
			shots.forEach((s, k) => {
				const d = f - k;
				if (Math.abs(d) >= 1) {
					s.style.opacity = '0';
					return;
				}
					const far = ramp(0.42, 0.58, Math.abs(d));
				const e = Math.sign(d) * far;
				s.style.opacity = (1 - far).toFixed(3);
				s.style.transform = `translate3d(0, ${(-e * 9).toFixed(2)}%, 0) scale(${(1 + (e < 0 ? 0.08 : -0.06) * far).toFixed(4)})`;
			});
			// The live client cannot be in two places: it dips out, changes route in the
			// dark at the halfway point, and comes back.
			const d = f - i;
			const dip = routes[i] ? ramp(0.43, 0.5, Math.abs(d)) : 1;
			frame.style.opacity = (1 - dip).toFixed(3);
			frame.style.translate = `0 ${(-Math.sign(d) * dip * 5).toFixed(2)}%`;
			frame.style.pointerEvents = dip > 0.5 ? 'none' : '';
			// A step with no route of its own (the Android app) shows its picture instead.
			if (el.classList.contains('is-still') !== !routes[i]) {
				el.classList.toggle('is-still', !routes[i]);
				if (live() && !driving()) say(idle());
			}
			caps.forEach((c, k) => {
				const dc = f - k;
				const far = ramp(0.41, 0.5, Math.abs(dc));
				c.style.opacity = (1 - far).toFixed(3);
				c.style.transform = `translate3d(0, ${(-Math.sign(dc) * far * 40).toFixed(1)}px, 0)`;
			});
		});
		// The frame itself: it arrives tilted back, leans with the scroll's speed and
		// the pointer, and a phone turns on its axis between one screen and the next.
		// Under the pointer it stands still, so what you click stays where it is.
		let still = 0;
		let flat = false;
		const pose = () => {
			if (calm) return;
			still = lerp(still, hover, 0.12);
			const [a, b] = range();
			const before = clamp((a - scroll.y) / (innerHeight * 0.9));
			const f = clamp((scroll.y - a) / Math.max(1, b - a)) * (n - 1);
			const lean = clamp(scroll.v * -0.12, -7, 7);
			const turn = phone ? Math.sin(f * Math.PI) * -22 * (Math.floor(f) % 2 ? -1 : 1) : 0;
			// The pointer only sways the pictures: a live client held at an angle is a blurred one.
			const sway = live() ? 0 : mouse.x * (phone ? 10 : 4) * (1 - still);
			const ry = sway + turn;
			// At rest there is no transform at all, and the frame is nudged onto whole pixels.
			if (before < 0.001 && Math.abs(lean) < 0.04 && Math.abs(ry) < 0.04) {
				if (flat) return;
				flat = true;
				holder.style.transform = 'none';
				const box = view.getBoundingClientRect();
				holder.style.transform = `translate(${(Math.round(box.left) - box.left).toFixed(3)}px, ${(Math.round(box.top) - box.top).toFixed(3)}px)`;
				return;
			}
			flat = false;
			holder.style.transform =
				`translate3d(0, ${(before * 60).toFixed(1)}px, 0) rotateX(${(before * 22 + lean).toFixed(2)}deg) ` +
				`rotateY(${ry.toFixed(2)}deg) scale(${(1 - before * 0.1).toFixed(4)})`;
		};
		dots.forEach((d, i) => d.addEventListener('click', () => reel.goto(i)));
		const reel = {
			el,
			frame,
			routes,
			show,
			load,
			pose,
			engage,
			y: (i) => {
				const [a, b] = range();
				return lerp(a, b, n > 1 ? i / (n - 1) : 0);
			},
			goto: (i) => scrollToY(reel.y(i)),
			index: () => Math.max(0, at)
		};
		return reel;
	});
	// The client is fetched once the page itself has loaded: the desktop one where a
	// 1440 px client is readable, the phone one always.
	const wide = () => innerWidth > 820;
	// ---- the first load: the hero's clients are fetched behind a bar, and the page
	// is shown once they have drawn, so nothing pops in afterwards. The showcases
	// further down fetch theirs right after, from the cache the hero filled.
	const loader = document.querySelector('[data-loader]');
	const reveal = () => {
		if (root.classList.contains('is-ready')) return;
		root.classList.remove('is-loading');
		root.classList.add('is-ready');
		measure();
		setTimeout(() => loader?.remove(), 900);
		setTimeout(() => wide() && reels[0]?.load(), 1200);
		setTimeout(() => reels[1]?.load(), wide() ? 3200 : 1200);
	};
	const loadDemos = () => {
		const mine = planes.filter((p) => !(narrowNow() && !p.phone));
		// The first client fetches the bundle; the others start once it has stopped
		// asking for files, and take theirs from the cache instead of the network.
		// A returning visitor already has the bundle: nothing to wait for.
		let primed = false;
		try {
			primed = localStorage.getItem('lain-demo') === '1';
		} catch {
			/* storage is off: prime as a first visit does */
		}
		for (const pl of primed ? mine : mine.slice(0, 1)) loadPlane(pl);
		let count = -1;
		let quiet = 0;
		const prime = setInterval(() => {
			let n = 0;
			let ready = true;
			try {
				const win = mine[0].frame.contentWindow;
				// The demo answers from its service worker: a client opened before the worker
				// has taken control asks the network for pictures that only the worker has.
				const sw = win.navigator.serviceWorker;
				ready = !sw || Boolean(sw.controller);
				n = ready && win.document.readyState === 'complete' ? win.performance.getEntriesByType('resource').length : 0;
			} catch {
				n = 0;
			}
			quiet = n > 8 && n === count ? quiet + 1 : 0;
			count = n;
			// Waited long enough (a worker that never takes control): go anyway.
			if (performance.now() - since > 6000) ready = true;
			if (!ready || (quiet < 3 && !mine[0].done)) return;
			clearInterval(prime);
			for (const pl of mine.slice(1)) loadPlane(pl);
			try {
				localStorage.setItem('lain-demo', '1');
			} catch {
				/* storage is off */
			}
		}, 60);
		const fill = loader?.querySelector('[data-loader-fill]');
		const pct = loader?.querySelector('[data-loader-pct]');
		const pictures = [...document.querySelectorAll('.plane img')];
		let fonts = false;
		(document.fonts?.ready ?? Promise.resolve()).then(() => (fonts = true));
		const since = performance.now();
		let shown = 0;
		const tick = () => {
			const waited = performance.now() - since;
			const parts = [fonts, ...pictures.map((img) => img.complete), ...mine.flatMap((pl) => [pl.asked, pl.done, pl.done])];
			const real = parts.filter(Boolean).length / parts.length;
			// The bar never stands still, and never claims more than it has.
			shown = Math.max(shown, real, Math.min(0.9, waited / 7000));
			if (fill) fill.style.transform = `scaleX(${shown.toFixed(3)})`;
			if (pct) pct.textContent = String(Math.min(99, Math.round(shown * 100)));
			// A client that will not come (no build beside the page, a dead network) does not hold the page.
			if (real >= 1 || waited > 8000) return reveal();
			requestAnimationFrame(tick);
		};
		tick();
	};
	loadDemos();

	function drive(reel, to) {
		reel.show(to);
		// The keys belong to the demo once it is on screen.
		reel.engage(true);
	}
	// Open a route of the live demo, on the step that shows it when there is one.
	function demoGo(route) {
		const reel = (wide() ? reels[0] : reels[1]) ?? reels[0];
		if (!reel) return;
		const i = reel.routes.indexOf(route);
		reel.goto(i >= 0 ? i : reel.index());
		drive(reel, route);
	}
	document.addEventListener('click', (e) => {
		const b = e.target.closest('[data-demo-go]');
		if (b) demoGo(b.dataset.demoGo);
	});

	// ---- a picture that starts as a tilted card and grows into the window ------
	const full = new Map();
	for (const el of document.querySelectorAll('[data-grow]')) {
		const art = el.querySelector('.grow-art');
		const img = art.querySelector('img');
		if (calm) continue;
		scene(
			() => [top(el) - innerHeight * 0.75, top(el) + el.offsetHeight - innerHeight],
			(p) => {
				const e = ramp(0.05, 0.72, p);
				art.style.transform = `rotateX(${lerp(34, 0, e).toFixed(2)}deg) scale(${lerp(0.46, 1, e).toFixed(4)})`;
				art.style.borderRadius = `${lerp(36, 0, e).toFixed(1)}px`;
				// On a phone the picture is already the shape of its frame: it is not zoomed inside it.
				if (innerWidth > 600) img.style.transform = `scale(${lerp(1.22, 1, e).toFixed(4)}) translate3d(0, ${lerp(-4, 0, e).toFixed(2)}%, 0)`;
				else img.style.transform = '';
				full.set(el, e > 0.9 && p < 0.97);
				document.body.classList.toggle('is-full', [...full.values()].some(Boolean));
			}
		);
	}

	// ---- the terminal types itself the first time it is seen -------------------
	for (const term of document.querySelectorAll('[data-term]')) {
		if (calm) continue;
		const lines = [...term.querySelectorAll('.ln')].map((ln) => {
			const cmd = ln.querySelector('.cmd');
			const text = cmd.textContent;
			return { ln, cmd, text };
		});
		term.classList.add('typed');
		term.addEventListener('seen', async () => {
			const wait = (ms) => new Promise((r) => setTimeout(r, ms));
			for (const { ln, cmd, text } of lines) {
				cmd.textContent = '';
				ln.classList.add('is-shown');
				cmd.classList.add('is-typing');
				await wait(260);
				for (const ch of text) {
					cmd.textContent += ch;
					await wait(18 + Math.random() * 34);
				}
				cmd.classList.remove('is-typing');
				ln.classList.add('is-done');
				await wait(240);
			}
		});
	}

	// ---- keys you can press: the cap for the key you hit goes down --------------
	const capsEl = document.querySelector('[data-caps]');
	const press = (e) => {
		if (!capsEl) return;
		const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
		for (const li of capsEl.children) {
			if (li.dataset.key !== key || ('mod' in li.dataset) !== (e.ctrlKey || e.metaKey)) continue;
			li.classList.add('is-down');
			setTimeout(() => li.classList.remove('is-down'), 220);
		}
	};
	addEventListener('keydown', press, true);

	// ---- chapters: the rail's place and the sky's colour ------------------------
	const sections = [...document.querySelectorAll('[data-section]')];
	const railLinks = new Map([...document.querySelectorAll('[data-rail] a')].map((a) => [a.getAttribute('href').slice(1), a]));
	const hue = { r: 126, g: 31, b: 79, tr: 126, tg: 31, tb: 79 };
	const spy = new IntersectionObserver(
		(entries) => {
			for (const e of entries) {
				if (!e.isIntersecting) continue;
				const link = railLinks.get(e.target.id);
				if (link) {
					for (const a of railLinks.values()) a.removeAttribute('aria-current');
					link.setAttribute('aria-current', 'true');
				}
				const h = getComputedStyle(e.target).getPropertyValue('--hue').split(',').map(Number);
				if (h.length === 3 && !h.some(Number.isNaN)) [hue.tr, hue.tg, hue.tb] = h;
			}
		},
		{ rootMargin: '-45% 0px -50% 0px' }
	);
	for (const s of sections) spy.observe(s);

	// ---- the sky: one fragment shader behind the page ---------------------------
	// Slow warped noise in the chapter's colour, three depths of blinking motes
	// that slide with the scroll, and a dither so nothing bands.
	const sky = (() => {
		const canvas = document.querySelector('[data-sky]');
		const gl = canvas?.getContext('webgl', { antialias: false, alpha: false, powerPreference: 'low-power' });
		if (!gl) return null;
		const make = (type, src) => {
			const s = gl.createShader(type);
			gl.shaderSource(s, src);
			gl.compileShader(s);
			return gl.getShaderParameter(s, gl.COMPILE_STATUS) ? s : null;
		};
		const vs = make(gl.VERTEX_SHADER, 'attribute vec2 p;void main(){gl_Position=vec4(p,0.,1.);}');
		const fs = make(
			gl.FRAGMENT_SHADER,
			`precision highp float;
uniform vec2 uRes; uniform float uTime; uniform float uScroll; uniform vec2 uMouse; uniform vec3 uHue; uniform float uHero;
float hash(vec2 p){ p=fract(p*vec2(123.34,456.21)); p+=dot(p,p+45.32); return fract(p.x*p.y); }
float noise(vec2 p){ vec2 i=floor(p), f=fract(p); f=f*f*(3.-2.*f);
  return mix(mix(hash(i),hash(i+vec2(1.,0.)),f.x), mix(hash(i+vec2(0.,1.)),hash(i+vec2(1.,1.)),f.x), f.y); }
float fbm(vec2 p){ float a=.5, s=0.; for(int i=0;i<5;i++){ s+=a*noise(p); p=p*2.03+vec2(17.3,9.1); a*=.5; } return s; }
float motes(vec2 uv, float scale, float t){ vec2 g=uv*scale; vec2 id=floor(g); vec2 f=fract(g)-.5;
  float h=hash(id); vec2 o=(vec2(hash(id+3.1),hash(id+7.7))-.5)*.7;
  o+=.08*vec2(sin(t*.5+h*30.),cos(t*.4+h*20.));
  float blink=.35+.65*(.5+.5*sin(t*(.5+h*1.6)+h*40.));
  return smoothstep(.034,.0,length(f-o))*step(.7,h)*blink; }
void main(){
  vec2 p=(gl_FragCoord.xy-.5*uRes)/uRes.y; vec2 m=uMouse;
  float t=uTime*.035;
  vec2 q=p*1.25+vec2(0.,-uScroll*.22);
  vec2 w=vec2(fbm(q+vec2(t,-t)), fbm(q+vec2(5.2,1.3)-t));
  float n=fbm(q+1.9*w+m*.35);
  float cloud=smoothstep(.32,.86,n);
  vec3 col=vec3(.047,.043,.047);
  col+=uHue*cloud*(.42+.34*uHero);
  col+=vec3(.71,.59,.56)*pow(n,5.)*.22;
  col*=1.-.5*dot(p*vec2(.75,1.05),p*vec2(.75,1.05));
  float s=motes(p+vec2(0.,-uScroll*.10)+m*.02,11.,uTime)*.55
         +motes(p+vec2(3.,-uScroll*.26)+m*.05,19.,uTime*1.3)*.4
         +motes(p+vec2(7.,-uScroll*.55)+m*.10,6.,uTime*.8)*.7;
  col+=vec3(.86,.74,.70)*s*(.45+.55*uHero);
  col+=(hash(gl_FragCoord.xy+fract(uTime))-.5)*.024;
  gl_FragColor=vec4(max(col,0.),1.);
}`
		);
		if (!vs || !fs) return null;
		const prog = gl.createProgram();
		gl.attachShader(prog, vs);
		gl.attachShader(prog, fs);
		gl.linkProgram(prog);
		if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return null;
		gl.useProgram(prog);
		gl.bindBuffer(gl.ARRAY_BUFFER, gl.createBuffer());
		gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
		const loc = gl.getAttribLocation(prog, 'p');
		gl.enableVertexAttribArray(loc);
		gl.vertexAttribPointer(loc, 2, gl.FLOAT, false, 0, 0);
		const u = Object.fromEntries(['uRes', 'uTime', 'uScroll', 'uMouse', 'uHue', 'uHero'].map((k) => [k, gl.getUniformLocation(prog, k)]));
		// Soft shapes: three quarters of the window's pixels is plenty.
		const size = () => {
			const k = Math.min(devicePixelRatio || 1, 1.5) * 0.75;
			canvas.width = Math.round(innerWidth * k);
			canvas.height = Math.round(innerHeight * k);
			gl.viewport(0, 0, canvas.width, canvas.height);
		};
		size();
		addEventListener('resize', size);
		return (now) => {
			gl.uniform2f(u.uRes, canvas.width, canvas.height);
			gl.uniform1f(u.uTime, calm ? 0 : now / 1000);
			gl.uniform1f(u.uScroll, scroll.y / innerHeight);
			gl.uniform2f(u.uMouse, mouse.x, -mouse.y);
			gl.uniform3f(u.uHue, hue.r / 255, hue.g / 255, hue.b / 255);
			gl.uniform1f(u.uHero, 1 - ramp(0.2, 1, heroP));
			gl.drawArrays(gl.TRIANGLES, 0, 3);
		};
	})();

	// ---- the loop ----------------------------------------------------------------
	const nav = document.getElementById('nav');
	let last = performance.now();
	const frame = (now) => {
		requestAnimationFrame(frame);
		if (document.hidden) return;
		const dt = Math.min(64, now - last) / 16.67;
		last = now;
		const before = scroll.y;
		if (scroll.smooth) {
			if (Math.abs(scroll.target - scroll.y) > 0.4) {
				scroll.y = lerp(scroll.y, scroll.target, 1 - (1 - 0.1) ** dt);
				scrollTo(0, scroll.y);
			} else scroll.y = scroll.target;
		} else scroll.y = scrollY;
		scroll.v = lerp(scroll.v, (scroll.y - before) / dt, 0.2);
		mouse.x = lerp(mouse.x, mouse.tx, 0.06);
		mouse.y = lerp(mouse.y, mouse.ty, 0.06);
		hue.r = lerp(hue.r, hue.tr, 0.04);
		hue.g = lerp(hue.g, hue.tg, 0.04);
		hue.b = lerp(hue.b, hue.tb, 0.04);
		for (const s of scenes) {
			const raw = clamp((scroll.y - s.a) / Math.max(1, s.b - s.a));
			const p = s.p < 0 || calm ? raw : Math.abs(raw - s.p) < 0.0004 ? raw : lerp(s.p, raw, s.ease * dt);
			if (p === s.p) continue;
			s.p = p;
			s.update(p);
		}
		drawPlanes(now);
		for (const r of reels) r.pose();
		sky?.(now);
		nav.classList.toggle('is-stuck', scroll.y > innerHeight * 1.25);
	};
	measure();
	new ResizeObserver(measure).observe(document.body);
	addEventListener('resize', measure);
	addEventListener('load', measure);
	requestAnimationFrame(frame);

	// ---- links inside the page ride the same scroll -------------------------------
	document.addEventListener('click', (e) => {
		const a = e.target.closest('a[href^="#"]');
		const to = a && document.getElementById(a.getAttribute('href').slice(1));
		if (!to) return;
		e.preventDefault();
		if (to.id === 'top') scrollToY(0);
		else jumpTo(to);
		history.replaceState(null, '', a.getAttribute('href'));
	});

	// J and K walk every stop: the top, each chapter, each screen of the showcases.
	const stops = () =>
		[
			0,
			...sections.map((s) => top(s.querySelector('.chapter-head') ?? s) - NAV - 40),
			...reels.flatMap((r) => r.routes.map((_, i) => r.y(i))),
			...[...document.querySelectorAll('[data-grow]')].map((g) => top(g) + g.offsetHeight - innerHeight)
		].sort((a, b) => a - b);
	const step = (by) => {
		const all = stops();
		const here = scroll.smooth ? scroll.target : scrollY;
		const next = by > 0 ? all.find((y) => y > here + 8) : all.findLast((y) => y < here - 8);
		if (next !== undefined) scrollToY(next);
	};

	// ---- install: one panel per tab ------------------------------------------------
	const install = document.querySelector('[data-install]');
	if (install) {
		tablist(install.querySelector('[role="tablist"]'), (tab) => {
			for (const p of install.querySelectorAll('[role="tabpanel"]')) p.hidden = p.id !== tab.getAttribute('aria-controls');
		});
	}

	// Copy buttons say so, briefly.
	const toastEl = document.querySelector('[data-toast]');
	let toastTimer = 0;
	const toast = (text) => {
		if (!toastEl) return;
		toastEl.textContent = text;
		toastEl.classList.add('is-on');
		clearTimeout(toastTimer);
		toastTimer = setTimeout(() => toastEl.classList.remove('is-on'), 2200);
	};
	async function copy(b) {
		if (!b) return;
		try {
			await navigator.clipboard.writeText(b.dataset.copy);
			toast(`Copied: ${b.dataset.copy}`);
		} catch {
			// No clipboard here: select the command so it can be copied by hand.
			const code = b.closest('.code')?.querySelector('code');
			if (code) getSelection()?.selectAllChildren(code);
			toast('Selected. Press Ctrl+C to copy');
			return;
		}
		if (b.dataset.label) return;
		b.dataset.label = b.innerHTML;
		b.textContent = 'Copied';
		b.classList.add('is-done');
		setTimeout(() => {
			b.innerHTML = b.dataset.label;
			delete b.dataset.label;
			b.classList.remove('is-done');
		}, 1400);
	}
	document.addEventListener('click', (e) => {
		const b = e.target.closest('[data-copy]');
		if (b) copy(b);
	});

	// ---- the palette: every chapter and action, the way Ctrl+K works in the client --
	const palette = document.getElementById('palette');
	const input = document.getElementById('palette-input');
	const list = document.getElementById('palette-list');
	const mainCopy = document.querySelector('[data-copy-main]');
	// C copies the command of the install tab that is showing (Docker when it has none).
	const currentCopy = () => document.querySelector('[data-install] [role="tabpanel"]:not([hidden]) [data-copy]') ?? mainCopy;
	const toInstall = () => jumpTo(document.getElementById('install'));
	const open = (url) => window.open(url, '_blank', 'noopener');
	const commands = [
		...sections.map((s) => ({ kind: 'chapter' in s.dataset ? 'Chapter' : 'Section', label: s.dataset.section, keys: s.id === 'install' ? ['I'] : undefined, run: () => jumpTo(s) })),
		{ kind: 'Action', label: 'Use the live client', keys: ['D'], run: openHero },
		{ kind: 'Action', label: 'Copy the install command', keys: ['C'], run: () => copy(currentCopy()) },
		{ kind: 'Demo', label: 'Open the library', run: () => demoGo('/library?view=all') },
		{ kind: 'Demo', label: 'Open a series', run: () => demoGo('/item/039a34b14ce22380') },
		{ kind: 'Demo', label: 'Open the player (Play has a surprise)', run: () => demoGo('/player/039a34b14ce22380') },
		{ kind: 'Demo', label: 'Open the reader', run: () => demoGo('/read/ab0fce996f9aa7f4') },
		{ kind: 'Demo', label: 'Open the transcoding settings', run: () => demoGo('/settings/transcoding') },
		{ kind: 'Demo', label: 'Open the plugins and their providers', run: () => demoGo('/settings/plugins') },
		{ kind: 'Link', label: 'Source on GitHub', run: () => open('https://github.com/enrell/lain') },
		{ kind: 'Link', label: 'Latest release', run: () => open('https://github.com/enrell/lain/releases/latest') },
		{ kind: 'Link', label: 'Changelog', run: () => open('https://github.com/enrell/lain/blob/main/CHANGELOG.md') },
		{ kind: 'Link', label: 'Docs', run: () => open('https://github.com/enrell/lain/tree/main/docs') },
		{ kind: 'Link', label: 'Report an issue', run: () => open('https://github.com/enrell/lain/issues') }
	];
	let shown = [];
	let at = 0;
	const mark = () => {
		[...list.children].forEach((li, i) => li.setAttribute('aria-selected', String(i === at)));
		list.children[at]?.scrollIntoView({ block: 'nearest' });
		input.setAttribute('aria-activedescendant', shown.length ? `cmd-${at}` : '');
	};
	const render = () => {
		const words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
		shown = commands.filter((c) => words.every((w) => `${c.kind} ${c.label}`.toLowerCase().includes(w)));
		at = 0;
		list.replaceChildren(
			...shown.map((c, i) => {
				const li = document.createElement('li');
				li.id = `cmd-${i}`;
				li.setAttribute('role', 'option');
				const kind = document.createElement('span');
				kind.className = 'kind';
				kind.textContent = c.kind;
				const label = document.createElement('span');
				label.textContent = c.label;
				li.append(kind, label);
				if (c.keys) {
					const keys = document.createElement('span');
					keys.className = 'keys';
					for (const k of c.keys) keys.append(Object.assign(document.createElement('kbd'), { textContent: k }));
					li.append(keys);
				}
				li.addEventListener('click', () => run(i));
				li.addEventListener('pointermove', () => {
					if (at !== i) {
						at = i;
						mark();
					}
				});
				return li;
			})
		);
		if (!shown.length) {
			const li = document.createElement('li');
			li.className = 'empty';
			li.textContent = 'Nothing here by that name.';
			list.append(li);
		}
		mark();
	};
	const run = (i) => {
		const c = shown[i];
		if (!c) return;
		palette.close();
		c.run();
	};
	const openPalette = () => {
		if (palette.open) return;
		input.value = '';
		render();
		palette.showModal();
	};
	input.addEventListener('input', render);
	palette.addEventListener('keydown', (e) => {
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			if (shown.length) at = (at + (e.key === 'ArrowDown' ? 1 : -1) + shown.length) % shown.length;
			mark();
		} else if (e.key === 'Enter') {
			e.preventDefault();
			run(at);
		}
	});
	// A click on the backdrop closes it, like Esc.
	palette.addEventListener('click', (e) => {
		if (e.target === palette) palette.close();
	});
	document.querySelector('[data-palette-open]')?.addEventListener('click', openPalette);

	// ---- keys. Typing in a field, or holding a modifier, is never a shortcut ---------
	addEventListener('keydown', (e) => {
		if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === 'k') {
			e.preventDefault();
			openPalette();
			return;
		}
		if (e.ctrlKey || e.metaKey || e.altKey || palette.open) return;
		const t = e.target;
		if (t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName))) return;
		const act = {
			'/': openPalette,
			'?': openPalette,
			j: () => step(1),
			k: () => step(-1),
			d: openHero,
			i: toInstall,
			c: () => copy(currentCopy())
		}[e.key.length === 1 ? e.key.toLowerCase() : e.key];
		if (!act) return;
		e.preventDefault();
		act();
	});

	// ---- the release line, the download size and the APK link follow GitHub -----------
	fetch('https://api.github.com/repos/enrell/lain/releases/latest', { headers: { Accept: 'application/vnd.github+json' } })
		.then((res) => (res.ok ? res.json() : null))
		.then((release) => {
			if (!release?.tag_name) return;
			const set = (sel, text) => document.querySelectorAll(sel).forEach((el) => (el.textContent = text));
			set('[data-release-version]', release.tag_name);
			const when = new Date(release.published_at);
			if (!Number.isNaN(when.getTime())) {
				set('[data-release-date]', when.toLocaleDateString('en-GB', { day: 'numeric', month: 'short', year: 'numeric', timeZone: 'UTC' }));
			}
			const assets = Array.isArray(release.assets) ? release.assets : [];
			const apk = assets.find((a) => a?.name?.endsWith('.apk') && a.browser_download_url);
			if (apk) document.querySelectorAll('[data-apk-link]').forEach((a) => (a.href = apk.browser_download_url));
			const linux = assets.find((a) => a?.name?.endsWith('_linux_amd64.tar.gz') && a.size > 0);
			if (linux) set('[data-linux-size-short]', `${(linux.size / 1e6).toFixed(1)} MB`);
		})
		.catch(() => {});
})();
