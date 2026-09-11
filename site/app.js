/* lain site — no dependencies, no trackers, nothing fetched at runtime. */
(() => {
	"use strict";

	const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
	const canHover = window.matchMedia("(hover: hover) and (pointer: fine)").matches;

	/* ------------------------------------------------------------- canvas -- */

	const canvas = document.getElementById("constellation");
	if (canvas && canvas.getContext) {
		const ctx = canvas.getContext("2d");
		let width = 0;
		let height = 0;
		let nodes = [];
		let pointer = { x: -10000, y: -10000 };
		let visible = true;
		let raf = 0;

		const seed = () => {
			const count = Math.min(96, Math.max(28, Math.round((width * height) / 26000)));
			nodes = Array.from({ length: count }, () => ({
				x: Math.random() * width,
				y: Math.random() * height,
				vx: (Math.random() - 0.5) * 0.14,
				vy: (Math.random() - 0.5) * 0.14,
				r: Math.random() * 1.3 + 0.5
			}));
		};

		const resize = () => {
			const dpr = Math.min(2, window.devicePixelRatio || 1);
			width = canvas.clientWidth;
			height = canvas.clientHeight;
			canvas.width = Math.round(width * dpr);
			canvas.height = Math.round(height * dpr);
			ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
			seed();
		};

		const draw = () => {
			ctx.clearRect(0, 0, width, height);
			const linkDist = Math.min(160, Math.max(96, width / 12));

			for (const n of nodes) {
				n.x += n.vx;
				n.y += n.vy;
				if (n.x < -20) n.x = width + 20;
				if (n.x > width + 20) n.x = -20;
				if (n.y < -20) n.y = height + 20;
				if (n.y > height + 20) n.y = -20;

				const dx = n.x - pointer.x;
				const dy = n.y - pointer.y;
				const d2 = dx * dx + dy * dy;
				if (d2 < 26000 && d2 > 1) {
					const f = (1 - d2 / 26000) * 0.35;
					const d = Math.sqrt(d2);
					n.x += (dx / d) * f;
					n.y += (dy / d) * f;
				}
			}

			for (let i = 0; i < nodes.length; i++) {
				const a = nodes[i];
				for (let j = i + 1; j < nodes.length; j++) {
					const b = nodes[j];
					const dx = a.x - b.x;
					const dy = a.y - b.y;
					const dist = Math.sqrt(dx * dx + dy * dy);
					if (dist < linkDist) {
						const alpha = (1 - dist / linkDist) * 0.16;
						ctx.strokeStyle = `rgba(122,162,247,${alpha.toFixed(3)})`;
						ctx.lineWidth = 1;
						ctx.beginPath();
						ctx.moveTo(a.x, a.y);
						ctx.lineTo(b.x, b.y);
						ctx.stroke();
					}
				}
			}

			ctx.fillStyle = "rgba(190,205,235,0.55)";
			for (const n of nodes) {
				ctx.beginPath();
				ctx.arc(n.x, n.y, n.r, 0, Math.PI * 2);
				ctx.fill();
			}
		};

		const loop = () => {
			if (visible) draw();
			raf = window.requestAnimationFrame(loop);
		};

		const start = () => {
			resize();
			if (reduced) {
				draw();
				return;
			}
			if (!raf) loop();
		};

		window.addEventListener("resize", resize, { passive: true });
		window.addEventListener(
			"pointermove",
			(e) => {
				const rect = canvas.getBoundingClientRect();
				pointer = { x: e.clientX - rect.left, y: e.clientY - rect.top };
			},
			{ passive: true }
		);
		window.addEventListener("pointerleave", () => {
			pointer = { x: -10000, y: -10000 };
		});

		if ("IntersectionObserver" in window) {
			new IntersectionObserver(
				(entries) => {
					visible = entries[0].isIntersecting;
				},
				{ threshold: 0 }
			).observe(canvas);
		}

		document.addEventListener("visibilitychange", () => {
			visible = !document.hidden;
		});

		start();
	}

	/* ------------------------------------------------------------ reveals -- */

	const revealables = document.querySelectorAll(".reveal");
	if ("IntersectionObserver" in window && !reduced) {
		const io = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (entry.isIntersecting) {
						entry.target.classList.add("in-view");
						io.unobserve(entry.target);
					}
				}
			},
			{ threshold: 0.14, rootMargin: "0px 0px -8% 0px" }
		);
		revealables.forEach((el) => io.observe(el));
	} else {
		revealables.forEach((el) => el.classList.add("in-view"));
	}

	/* ---------------------------------------------------------------- nav -- */

	const nav = document.getElementById("nav");
	const progress = document.getElementById("progress");
	const onScroll = () => {
		const y = window.scrollY || 0;
		if (nav) nav.classList.toggle("is-scrolled", y > 12);
		if (progress) {
			const max = document.documentElement.scrollHeight - window.innerHeight;
			progress.style.transform = `scaleX(${max > 0 ? Math.min(1, y / max) : 0})`;
		}
	};
	window.addEventListener("scroll", onScroll, { passive: true });
	onScroll();

	const menuButton = document.getElementById("navMenu");
	const mobileNav = document.getElementById("mobileNav");
	if (menuButton && mobileNav) {
		menuButton.addEventListener("click", () => {
			const open = menuButton.getAttribute("aria-expanded") === "true";
			menuButton.setAttribute("aria-expanded", String(!open));
			mobileNav.hidden = open;
		});
		mobileNav.querySelectorAll("a").forEach((link) =>
			link.addEventListener("click", () => {
				menuButton.setAttribute("aria-expanded", "false");
				mobileNav.hidden = true;
			})
		);
	}

	/* ----------------------------------------------------------- spotlight -- */

	if (canHover) {
		document.querySelectorAll("[data-spot]").forEach((el) => {
			el.addEventListener(
				"pointermove",
				(e) => {
					const rect = el.getBoundingClientRect();
					el.style.setProperty("--mx", `${e.clientX - rect.left}px`);
					el.style.setProperty("--my", `${e.clientY - rect.top}px`);
				},
				{ passive: true }
			);
		});
	}

	/* ---------------------------------------------------------- registry --- */

	const genCount = document.getElementById("genCount");
	const chips = Array.from(document.querySelectorAll(".registry-chips .chip"));
	if (chips.length && !reduced) {
		chips.forEach((chip) => chip.classList.add("is-active"));
		let generation = 1;
		window.setInterval(() => {
			const shuffled = [...chips].sort(() => Math.random() - 0.5);
			const activeCount = 3 + Math.floor(Math.random() * 2);
			chips.forEach((chip) => chip.classList.remove("is-active"));
			shuffled.slice(0, activeCount).forEach((chip) => chip.classList.add("is-active"));
			if (genCount && Math.random() > 0.55) {
				generation += 1;
				genCount.textContent = String(generation);
			}
		}, 2200);
	}

	/* -------------------------------------------------------------- tabs --- */

	const tabs = Array.from(document.querySelectorAll(".tab"));
	const termTitle = document.getElementById("termTitle");
	const panels = Array.from(document.querySelectorAll(".tab-panel"));

	tabs.forEach((tab) => {
		tab.addEventListener("click", () => {
			tabs.forEach((t) => {
				const active = t === tab;
				t.classList.toggle("is-active", active);
				t.setAttribute("aria-selected", String(active));
			});
			panels.forEach((panel) => {
				panel.hidden = panel.id !== `panel-${tab.dataset.tab}`;
			});
			if (termTitle) termTitle.textContent = tab.dataset.tab;
			window.requestAnimationFrame(() => typeInVisiblePanel());
		});
	});

	/* -------------------------------------------------------------- typing -- */

	const typed = new WeakSet();
	const type = (pre) => {
		if (typed.has(pre)) return;
		typed.add(pre);
		const text = pre.dataset.command || "";
		if (reduced) {
			pre.textContent = text;
			return;
		}
		pre.textContent = "";
		const caret = document.createElement("span");
		caret.className = "caret";
		pre.appendChild(caret);
		let i = 0;
		const tick = () => {
			if (i >= text.length) {
				caret.remove();
				return;
			}
			const chunk = Math.random() > 0.85 ? 2 : 1;
			caret.insertAdjacentText("beforebegin", text.slice(i, i + chunk));
			i += chunk;
			window.setTimeout(tick, 12 + Math.random() * 26);
		};
		tick();
	};

	const typeInVisiblePanel = () => {
		const panel = panels.find((p) => !p.hidden);
		if (!panel) return;
		const pre = panel.querySelector("pre[data-command]");
		if (!pre) return;
		if (reduced || !("IntersectionObserver" in window)) {
			type(pre);
			return;
		}
		if (pre.getBoundingClientRect().top < window.innerHeight * 1.1) type(pre);
	};

	if ("IntersectionObserver" in window) {
		const typeObserver = new IntersectionObserver(
			(entries) => {
				for (const entry of entries) {
					if (entry.isIntersecting) {
						type(entry.target);
						typeObserver.unobserve(entry.target);
					}
				}
			},
			{ threshold: 0.3 }
		);
		document.querySelectorAll("pre[data-command]").forEach((pre) => typeObserver.observe(pre));
	} else {
		document.querySelectorAll("pre[data-command]").forEach(type);
	}

	/* -------------------------------------------------------------- copy --- */

	document.querySelectorAll(".copy").forEach((button) => {
		button.addEventListener("click", async () => {
			const panel = panels.find((p) => !p.hidden);
			const pre = panel ? panel.querySelector("pre[data-command]") : null;
			const text = button.dataset.copy || (pre ? pre.dataset.command : "");
			try {
				if (navigator.clipboard && window.isSecureContext) {
					await navigator.clipboard.writeText(text);
				} else {
					const area = document.createElement("textarea");
					area.value = text;
					area.style.position = "fixed";
					area.style.opacity = "0";
					document.body.appendChild(area);
					area.select();
					document.execCommand("copy");
					area.remove();
				}
				button.classList.add("is-copied");
				button.textContent = "copied";
				window.setTimeout(() => {
					button.classList.remove("is-copied");
					button.textContent = "copy";
				}, 1500);
			} catch {
				/* clipboard denied: leave the command selectable */
			}
		});
	});
})();
