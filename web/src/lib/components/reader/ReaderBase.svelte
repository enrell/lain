<script lang="ts">
	/*
	 * The shared reading engine (D-085): page fetching, paged and strip
	 * views, direction-aware navigation, preloading, progress reporting
	 * and the end-of-chapter card. MangaReader and ComicReader adapt it —
	 * defaults through a profile, and their own toolbar controls through
	 * the `toolbar` snippet — but never reimplement any of this.
	 */
	import { tick, untrack, type Snippet } from 'svelte';
	import { goto } from '$app/navigation';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import ChevronLeft from '@lucide/svelte/icons/chevron-left';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import CircleHelp from '@lucide/svelte/icons/circle-help';
	import Maximize from '@lucide/svelte/icons/maximize';
	import Minimize from '@lucide/svelte/icons/minimize';
	import { api } from '$lib/api';
	import type { CatalogItem, ReaderView } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import {
		buildSpreads,
		keyAction,
		spreadContaining,
		tapAction,
		visualOrder,
		type NavAction
	} from '$lib/reader/layout';
	import type { ReaderProfile } from '$lib/reader/profile';
	import type { ReaderSettings } from '$lib/reader/settings.svelte';

	interface Neighbour {
		href: string;
		label: string;
	}

	let {
		item,
		view,
		startAt,
		settings,
		profile,
		label,
		backHref,
		prev = null,
		next = null,
		onPage,
		toolbar
	}: {
		item: CatalogItem;
		view: ReaderView;
		startAt: number;
		settings: ReaderSettings;
		profile: ReaderProfile;
		/** The file's name in this kind's vocabulary, e.g. "Vol 3" or "Issue 4". */
		label: string;
		backHref: string;
		prev?: Neighbour | null;
		next?: Neighbour | null;
		onPage?: (page: number, total: number) => void;
		/** Adaptation-specific controls, rendered in the top bar. */
		toolbar?: Snippet;
	} = $props();

	const pages = $derived(view.pages);
	const total = $derived(pages.length);

	// svelte-ignore state_referenced_locally
	let current = $state(Math.min(Math.max(startAt, 0), Math.max(view.pages.length - 1, 0)));
	let ui = $state(true);
	let atEnd = $state(false);
	let innerWidth = $state(1280);
	let innerHeight = $state(800);
	let root = $state<HTMLDivElement>();
	let stage = $state<HTMLDivElement>();
	let strip = $state<HTMLDivElement>();
	let fullscreen = $state(false);
	let help = $state(false);
	// Go-to-page prompt (G): a number field, Enter jumps, Escape closes.
	let jumping = $state(false);
	let jumpValue = $state('');
	let jumpInput = $state<HTMLInputElement>();
	let endCard = $state<HTMLDivElement>();
	// Page turns are rate-limited so a held key, or a burst of them, can
	// never skip pages faster than the eye follows.
	const TURN_GAP_MS = 110;
	let lastTurn = 0;

	// Two pages only make sense on a landscape viewport.
	const dualActive = $derived(
		settings.mode === 'paged' && settings.dual && innerWidth > innerHeight
	);
	const spreads = $derived(buildSpreads(pages, { dual: dualActive, coverAlone: settings.coverAlone }));
	const spreadIdx = $derived(spreadContaining(spreads, current));
	const shown = $derived(visualOrder(spreads[spreadIdx] ?? [], settings.direction));
	const reading = $derived([...shown].sort((a, b) => a - b));
	const counter = $derived(
		reading.length === 0
			? ''
			: reading.length > 1
				? `${reading[0] + 1}–${reading[reading.length - 1] + 1} / ${total}`
				: `${reading[0] + 1} / ${total}`
	);

	const unit = $derived(profile.kind === 'manga' ? 'volume' : 'issue');
	const shortcuts = $derived<[string[], string][]>([
		[settings.mode === 'paged' && settings.direction === 'rtl' ? ['←'] : ['→'], 'Next page'],
		[settings.mode === 'paged' && settings.direction === 'rtl' ? ['→'] : ['←'], 'Previous page'],
		...(settings.mode === 'strip' ? [[['↑', '↓'], 'Scroll'] as [string[], string]] : []),
		[['Space', 'Shift+Space'], 'Next / previous page'],
		[['Home', 'End'], 'First / last page'],
		[['G'], 'Go to page'],
		[['N', ']'], 'Next ' + unit],
		[['P', '['], 'Previous ' + unit],
		[['B', 'Backspace'], 'Back to the title'],
		[['M'], profile.kind === 'manga' ? 'Paged / webtoon strip' : 'Paged / continuous scroll'],
		...(profile.controls.fit ? [[['W'], 'Fit page / full width'] as [string[], string]] : []),
		...(profile.controls.direction ? [[['R'], 'Flip reading direction'] as [string[], string]] : []),
		...(profile.controls.dual ? [[['D'], 'Two-page spreads'] as [string[], string]] : []),
		...(profile.controls.zoom ? [[['+', '-', '0'], 'Zoom in / out / reset'] as [string[], string]] : []),
		[['H'], 'Show or hide the bars'],
		[['F'], 'Fullscreen'],
		[['?'], 'This list']
	]);

	const kbd = 'shrink-0 rounded-md border border-line bg-background px-1.5 py-0.5 font-mono text-[10px] font-normal';

	const src = (i: number): string => api.reader.pageUrl(item.id, i, session.token);

	function go(action: NavAction): void {
		if (!action) return;
		if (action === 'toggle-ui') {
			ui = !ui;
			return;
		}
		if (settings.mode === 'strip') {
			scrollStrip(action);
			return;
		}
		if (action === 'next' || action === 'prev') {
			const now = performance.now();
			if (now - lastTurn < TURN_GAP_MS) return;
			lastTurn = now;
		}
		if (action === 'first') return jump(0);
		if (action === 'last') return jump(total - 1);
		if (action === 'next') {
			if (spreadIdx >= spreads.length - 1) {
				atEnd = true;
				ui = true;
				return;
			}
			jump(spreads[spreadIdx + 1][0]);
		} else {
			if (atEnd) {
				atEnd = false;
				return;
			}
			if (spreadIdx > 0) jump(spreads[spreadIdx - 1][0]);
		}
	}

	function jump(page: number): void {
		current = Math.min(Math.max(page, 0), Math.max(total - 1, 0));
		atEnd = false;
		ui = false;
		if (stage) stage.scrollTo({ top: 0, left: 0 });
	}

	// The strip moves one page per turn: the target page is looked up, not
	// estimated from a scroll distance, so lazy-loaded heights cannot skew it.
	function scrollStrip(action: NavAction): void {
		if (!strip) return;
		if (action === 'first') return void strip.scrollTo({ top: 0 });
		if (action === 'last') return void strip.scrollTo({ top: strip.scrollHeight });
		if (action !== 'next' && action !== 'prev') return;
		const now = performance.now();
		if (now - lastTurn < TURN_GAP_MS) return;
		lastTurn = now;
		if (action === 'next' && current >= total - 1) {
			atEnd = true;
			ui = true;
			return;
		}
		const target = Math.min(Math.max(current + (action === 'next' ? 1 : -1), 0), total - 1);
		strip.querySelector<HTMLElement>(`[data-index="${target}"]`)?.scrollIntoView({ block: 'start' });
		current = target;
	}

	function onStageClick(e: MouseEvent): void {
		if (!stage) return;
		const rect = stage.getBoundingClientRect();
		const action = tapAction(e.clientX - rect.left, rect.width, settings.direction);
		// A zoomed page is panned, not turned: only the centre still toggles the bars.
		if (settings.zoom > 1 && action !== 'toggle-ui') return;
		go(action);
	}

	function openChapter(target: Neighbour | null): void {
		if (target) void goto(target.href);
	}

	function openJump(): void {
		jumpValue = String(current + 1);
		jumping = true;
		void tick().then(() => jumpInput?.select());
	}

	function submitJump(): void {
		const n = Number.parseInt(jumpValue, 10);
		jumping = false;
		if (!Number.isFinite(n)) return;
		const page = Math.min(Math.max(n, 1), total) - 1;
		if (settings.mode === 'strip') {
			current = page;
			strip?.querySelector<HTMLElement>(`[data-index="${page}"]`)?.scrollIntoView({ block: 'start' });
		} else jump(page);
	}

	// The end card is a modal: focus lands on its primary action and Tab
	// cycles inside it, so it is fully usable without a mouse.
	$effect(() => {
		if (!atEnd || !endCard) return;
		void tick().then(() => endCard?.querySelector<HTMLElement>('[data-primary]')?.focus());
	});

	function trapTab(e: KeyboardEvent, box: HTMLElement | undefined): void {
		if (!box) return;
		const items = [...box.querySelectorAll<HTMLElement>('a[href],button:not([disabled])')];
		if (items.length === 0) return;
		const first = items[0];
		const last = items[items.length - 1];
		const active = document.activeElement;
		if (!box.contains(active)) {
			e.preventDefault();
			first.focus();
		} else if (e.shiftKey && active === first) {
			e.preventDefault();
			last.focus();
		} else if (!e.shiftKey && active === last) {
			e.preventDefault();
			first.focus();
		}
	}

	// Keys while the end card is open. The reading direction still means
	// something: "next" again opens the next file, "previous" goes back to
	// the last page.
	function onEndKey(e: KeyboardEvent): void {
		const k = e.key.toLowerCase();
		if (e.key === 'Tab') return trapTab(e, endCard);
		if (e.key === 'Escape') {
			e.preventDefault();
			atEnd = false;
			return;
		}
		if (e.key === 'Enter' || e.key === ' ') return; // activates the focused action
		if (k === 'n' || e.key === ']') {
			e.preventDefault();
			return openChapter(next);
		}
		if (k === 'p' || e.key === '[') {
			e.preventDefault();
			return openChapter(prev);
		}
		if (k === 'b' || e.key === 'Backspace') {
			e.preventDefault();
			return void goto(backHref);
		}
		const action = keyAction(e.key, e.shiftKey, settings.mode === 'strip' ? 'ltr' : settings.direction);
		const stripNext = settings.mode === 'strip' && (e.key === 'ArrowDown' || e.key === 'ArrowRight');
		if (action === 'next' || stripNext) {
			e.preventDefault();
			if (next) openChapter(next);
			return;
		}
		if (action === 'prev' || (settings.mode === 'strip' && e.key === 'ArrowLeft')) {
			e.preventDefault();
			atEnd = false;
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		if (help) {
			if (e.key === 'Escape' || e.key === '?') {
				e.preventDefault();
				help = false;
			}
			return;
		}
		if (jumping) return; // the page field owns the keyboard
		if (atEnd) return onEndKey(e);
		const t = e.target as HTMLElement | null;
		const tag = t?.tagName;
		if (tag === 'TEXTAREA' || tag === 'SELECT' || (tag === 'INPUT' && (t as HTMLInputElement).type !== 'range')) return;
		// Space and Enter activate a focused button or link; they must not also turn the page.
		if ((e.key === ' ' || e.key === 'Enter') && (tag === 'BUTTON' || tag === 'A')) return;
		if (e.key === '?') return void (help = true);
		const k = e.key.toLowerCase();
		if (k === 'g') {
			e.preventDefault();
			return openJump();
		}
		if (k === 'n' || e.key === ']') return openChapter(next);
		if (k === 'p' || e.key === '[') return openChapter(prev);
		if (k === 'b' || e.key === 'Backspace') {
			e.preventDefault();
			return void goto(backHref);
		}
		if (k === 'm') return settings.setMode(settings.mode === 'strip' ? 'paged' : 'strip');
		if (profile.controls.fit && k === 'w' && settings.mode === 'paged')
			return settings.setFit(settings.fit === 'contain' ? 'width' : 'contain');
		if (e.key === 'f') return void toggleFullscreen();
		if (e.key === 'h') return void (ui = !ui);
		if (profile.controls.zoom && settings.mode === 'paged') {
			if (e.key === '+' || e.key === '=') return void settings.setZoom(settings.zoom + 0.25);
			if (e.key === '-') return void settings.setZoom(settings.zoom - 0.25);
			if (e.key === '0') return void settings.setZoom(1);
		}
		if (profile.controls.dual && e.key === 'd') return void settings.toggleDual();
		if (profile.controls.direction && e.key === 'r') return void settings.toggleDirection();

		const strip_ = settings.mode === 'strip';
		// The strip is vertical: left/right step a page, up/down scroll a little.
		let action: NavAction;
		if (strip_ && e.key === 'ArrowRight') action = 'next';
		else if (strip_ && e.key === 'ArrowLeft') action = 'prev';
		else if (strip_ && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
			e.preventDefault();
			strip?.scrollBy({ top: (e.key === 'ArrowDown' ? 1 : -1) * strip.clientHeight * 0.4 });
			return;
		} else action = keyAction(e.key, e.shiftKey, settings.direction);
		if (!action) return;
		// A zoomed page scrolls with the arrows instead of turning.
		if (settings.zoom > 1 && !strip_ && (e.key.startsWith('Arrow') || e.key.startsWith('Page'))) return;
		e.preventDefault();
		// A slider that keeps focus would also move itself on the same key.
		if (t instanceof HTMLInputElement) t.blur();
		go(action);
	}

	async function toggleFullscreen(): Promise<void> {
		try {
			if (document.fullscreenElement) await document.exitFullscreen();
			else await root?.requestFullscreen();
		} catch {
			/* Fullscreen is a convenience; a refusal changes nothing. */
		}
	}

	// Report every page change; the route owns the throttling.
	$effect(() => {
		onPage?.(current, total);
	});

	// Warm the next spreads so a page turn never waits on the archive.
	$effect(() => {
		if (settings.mode !== 'paged') return;
		for (const s of spreads.slice(spreadIdx + 1, spreadIdx + 3)) {
			for (const i of s) new Image().src = src(i);
		}
	});

	// Entering strip mode lands on the current page. Reads current
	// untracked, or every scroll would re-run this and fight the user.
	$effect(() => {
		if (settings.mode !== 'strip' || !strip) return;
		const at = untrack(() => current);
		void tick().then(() => {
			strip?.querySelector<HTMLElement>(`[data-index="${at}"]`)?.scrollIntoView({ block: 'start' });
		});
	});

	let scrollFrame = 0;
	function onStripScroll(): void {
		if (scrollFrame || !strip) return;
		scrollFrame = requestAnimationFrame(() => {
			scrollFrame = 0;
			if (!strip) return;
			const probe = strip.scrollTop + strip.clientHeight * 0.3;
			let found = 0;
			for (const el of strip.querySelectorAll<HTMLElement>('[data-index]')) {
				if (el.offsetTop <= probe) found = Number(el.dataset.index);
				else break;
			}
			if (found !== current) current = found;
			const bottom = strip.scrollTop + strip.clientHeight >= strip.scrollHeight - 4;
			if (bottom && current !== total - 1) current = total - 1;
		});
	}

	function seek(e: Event): void {
		const input = e.currentTarget as HTMLInputElement;
		const v = Number(input.value) - 1;
		if (settings.mode === 'strip') {
			current = v;
			strip?.querySelector<HTMLElement>(`[data-index="${v}"]`)?.scrollIntoView({ block: 'start' });
		} else {
			jump(v);
			ui = true;
		}
	}

	// Sizing in paged mode. Zoom 1 fits the viewport; more than 1 makes
	// the page bigger than it and the stage scrolls to pan.
	function pageStyle(count: number): string {
		const z = settings.zoom;
		if (settings.fit === 'width') return `width:${(100 / count) * z}vw;max-width:none;height:auto`;
		if (z > 1) return `height:${z * 100}dvh;max-height:none;max-width:none;width:auto`;
		return `max-height:100dvh;max-width:${100 / count}vw;width:auto;height:auto`;
	}
</script>

<svelte:window bind:innerWidth bind:innerHeight onkeydowncapture={onKey} />
<svelte:document onfullscreenchange={() => (fullscreen = !!document.fullscreenElement)} />

<div
	bind:this={root}
	class="group/reader fixed inset-0 z-40 flex select-none flex-col bg-[color-mix(in_srgb,var(--color-background)_72%,black)] text-foreground"
>
	<!-- Top bar: the same glass panel language as the rest of Lain. -->
	<header
		class="pointer-events-none absolute inset-x-0 top-0 z-20 p-3 transition duration-300 ease-[var(--ease-out-quick)] {ui
			? ''
			: '-translate-y-[130%] opacity-0'}"
	>
		<div
			class="pointer-events-auto mx-auto flex max-w-5xl items-center gap-2 rounded-2xl border border-line/70 bg-background/80 p-2 shadow-[0_18px_45px_rgba(0,0,0,0.4)] backdrop-blur-xl"
		>
			<a
				href={backHref}
				class="inline-flex size-9 shrink-0 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
				aria-label="Back to {item.title}"
			>
				<ArrowLeft class="size-[18px]" aria-hidden="true" />
			</a>
			<div class="min-w-0 flex-1 px-1">
				<p class="truncate font-mono text-[10px] font-semibold uppercase tracking-[0.22em] text-accent">{label}</p>
				<p class="truncate text-sm font-semibold tracking-[-0.01em] text-foreground">{item.title}</p>
			</div>
			{#if counter}
				<p class="hidden shrink-0 px-2 font-mono text-xs tabular-nums text-muted sm:block" aria-live="polite">{counter}</p>
			{/if}
			<div class="flex shrink-0 items-center gap-0.5 rounded-xl border border-line/60 bg-surface/60 p-0.5">
				{@render toolbar?.()}
			</div>
			<div class="flex shrink-0 items-center gap-0.5">
				<button
					type="button"
					class="inline-flex size-9 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
					aria-label={fullscreen ? 'Exit fullscreen (F)' : 'Fullscreen (F)'}
					onclick={toggleFullscreen}
				>
					{#if fullscreen}<Minimize class="size-4" aria-hidden="true" />{:else}<Maximize class="size-4" aria-hidden="true" />{/if}
				</button>
				<button
					type="button"
					class="inline-flex size-9 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground {help
						? 'bg-accent/15 text-accent'
						: ''}"
					aria-label="Keyboard shortcuts (?)"
					aria-pressed={help}
					onclick={() => (help = !help)}
				>
					<CircleHelp class="size-4" aria-hidden="true" />
				</button>
			</div>
		</div>
	</header>

	{#if settings.mode === 'paged'}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div
			bind:this={stage}
			class="relative flex min-h-0 flex-1 overflow-auto"
			data-reader-stage
			data-direction={settings.direction}
			onclick={onStageClick}
		>
			<div class="m-auto flex {settings.fit === 'width' ? 'items-start' : 'items-center'} justify-center">
				{#each shown as i (i)}
					<img
						src={src(i)}
						alt="Page {i + 1}"
						draggable="false"
						class="block object-contain shadow-[0_24px_70px_rgba(0,0,0,0.55)]"
						style={pageStyle(shown.length)}
					/>
				{/each}
			</div>
		</div>
		<!-- Edge cues: which way "next" lies follows the reading direction. -->
		<div class="pointer-events-none absolute inset-y-0 left-0 z-10 hidden w-24 items-center justify-start pl-4 md:flex" aria-hidden="true">
			<span class="flex size-11 items-center justify-center rounded-full border border-line/60 bg-background/70 text-foreground opacity-0 backdrop-blur transition-opacity duration-200 group-hover/reader:opacity-70">
				<ChevronLeft class="size-5" />
			</span>
		</div>
		<div class="pointer-events-none absolute inset-y-0 right-0 z-10 hidden w-24 items-center justify-end pr-4 md:flex" aria-hidden="true">
			<span class="flex size-11 items-center justify-center rounded-full border border-line/60 bg-background/70 text-foreground opacity-0 backdrop-blur transition-opacity duration-200 group-hover/reader:opacity-70">
				<ChevronRight class="size-5" />
			</span>
		</div>
	{:else}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div
			bind:this={strip}
			class="min-h-0 flex-1 overflow-y-auto"
			data-reader-strip
			onscroll={onStripScroll}
			onclick={() => (ui = !ui)}
		>
			<div class="mx-auto flex max-w-3xl flex-col">
				{#each pages as p (p.index)}
					<img
						src={src(p.index)}
						alt="Page {p.index + 1}"
						data-index={p.index}
						loading="lazy"
						draggable="false"
						class="block w-full"
						style={p.width > 0 && p.height > 0 ? `aspect-ratio:${p.width}/${p.height}` : 'min-height:60dvh'}
					/>
				{/each}
				<div class="flex flex-col items-center gap-3 px-6 py-24 text-center">
					<p class="font-mono text-[10px] uppercase tracking-[0.26em] text-muted">End of {label}</p>
					{@render neighbours()}
				</div>
			</div>
		</div>
	{/if}

	<!-- Page counter that stays when the bars are away. -->
	{#if counter}
		<p
			class="pointer-events-none absolute bottom-3 left-1/2 z-10 -translate-x-1/2 rounded-full border border-line/50 bg-background/60 px-3 py-1 font-mono text-[10px] tabular-nums tracking-[0.14em] text-muted backdrop-blur transition-opacity duration-300 {ui
				? 'opacity-0'
				: 'opacity-100'}"
		>
			{counter}
		</p>
	{/if}

	{#if help}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="absolute inset-0 z-30 flex items-center justify-center bg-black/55 p-6 backdrop-blur-sm" onclick={() => (help = false)}>
			<div
				class="w-full max-w-md rounded-2xl border border-line bg-surface p-6 shadow-2xl"
				role="dialog"
				tabindex="-1"
				aria-label="Keyboard shortcuts"
				onclick={(e) => e.stopPropagation()}
			>
				<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent">Shortcuts</p>
				<dl class="mt-4 grid grid-cols-[auto_1fr] gap-x-5 gap-y-2.5 text-sm">
					{#each shortcuts as [keys, what] (what)}
						<dt class="flex flex-wrap gap-1">
							{#each keys as k (k)}
								<kbd class="rounded-md border border-line bg-background px-1.5 py-0.5 font-mono text-[11px] text-foreground">{k}</kbd>
							{/each}
						</dt>
						<dd class="text-muted">{what}</dd>
					{/each}
				</dl>
			</div>
		</div>
	{/if}

	{#if jumping}
		<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
		<div class="absolute inset-0 z-30 flex items-start justify-center bg-black/40 p-6 pt-[18vh]" onclick={() => (jumping = false)}>
			<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
			<form
				class="w-full max-w-xs rounded-2xl border border-line bg-surface p-4 shadow-2xl"
				onclick={(e) => e.stopPropagation()}
				onsubmit={(e) => {
					e.preventDefault();
					submitJump();
				}}
			>
				<label class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent" for="reader-jump">Go to page</label>
				<div class="mt-3 flex items-center gap-2">
					<input
						id="reader-jump"
						bind:this={jumpInput}
						bind:value={jumpValue}
						inputmode="numeric"
						autocomplete="off"
						class="h-10 w-full rounded-lg border border-line bg-background px-3 font-mono text-sm tabular-nums text-foreground outline-none focus:border-accent"
						onkeydown={(e) => {
							if (e.key === 'Escape') {
								e.preventDefault();
								jumping = false;
							}
						}}
					/>
					<span class="shrink-0 font-mono text-xs tabular-nums text-muted">/ {total}</span>
				</div>
				<p class="mt-2 font-mono text-[10px] text-muted"><kbd>Enter</kbd> jump · <kbd>Esc</kbd> cancel</p>
			</form>
		</div>
	{/if}

	{#if atEnd}
		<div class="absolute inset-0 z-20 flex items-center justify-center bg-black/65 p-6 backdrop-blur-sm">
			<div
				bind:this={endCard}
				role="dialog"
				aria-modal="true"
				aria-label="End of {label}"
				tabindex="-1"
				class="w-full max-w-sm rounded-2xl border border-line bg-surface p-6 text-center shadow-2xl"
			>
				<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent">End</p>
				<p class="mt-2 text-xl font-semibold tracking-[-0.02em] text-foreground">{label}</p>
				<p class="mt-1 text-sm text-muted">{item.title}</p>
				<div class="mt-6 flex flex-col gap-2">
					{@render neighbours()}
					<button
						type="button"
						data-primary={next ? undefined : true}
						class="flex items-center justify-between gap-3 rounded-lg px-4 py-2 text-sm text-muted transition-colors hover:bg-surface-hover hover:text-foreground focus-visible:bg-surface-hover focus-visible:text-foreground"
						onclick={() => (atEnd = false)}
					>
						<span>Keep reading this {unit}</span><kbd class={kbd}>Esc</kbd>
					</button>
				</div>
			</div>
		</div>
	{/if}

	<!-- Progress: reads in the same direction as the pages. -->
	<footer
		class="pointer-events-none absolute inset-x-0 bottom-0 z-20 p-3 transition duration-300 ease-[var(--ease-out-quick)] {ui
			? ''
			: 'translate-y-[130%] opacity-0'}"
	>
		<div
			class="pointer-events-auto mx-auto flex max-w-3xl items-center gap-3 rounded-2xl border border-line/70 bg-background/80 px-4 py-3 shadow-[0_18px_45px_rgba(0,0,0,0.4)] backdrop-blur-xl"
			dir={settings.mode === 'paged' ? settings.direction : 'ltr'}
		>
			<span class="font-mono text-[10px] tabular-nums text-muted">1</span>
			<input
				type="range"
				min="1"
				max={Math.max(total, 1)}
				value={current + 1}
				oninput={seek}
				onpointerup={(e) => e.currentTarget.blur()}
				tabindex="-1"
				aria-label="Page"
				class="reader-range"
				style:--fill="{total > 1 ? (current / (total - 1)) * 100 : 100}%"
				style:--to={settings.mode === 'paged' && settings.direction === 'rtl' ? 'left' : 'right'}
			/>
			<span class="font-mono text-[10px] tabular-nums text-muted">{total}</span>
		</div>
	</footer>
</div>

{#snippet neighbours()}
	{#if next}
		<a
			href={next.href}
			data-primary
			class="flex items-center justify-between gap-3 rounded-lg bg-accent px-4 py-2.5 text-sm font-semibold text-accent-fg transition-colors hover:bg-accent-hover focus-visible:outline-offset-2"
		>
			<span>Next: {next.label}</span><kbd class="{kbd} border-accent-fg/30 bg-accent-fg/10 text-accent-fg">Enter</kbd>
		</a>
	{/if}
	{#if prev}
		<a href={prev.href} class="flex items-center justify-between gap-3 rounded-lg px-4 py-2 text-sm text-muted transition-colors hover:bg-surface-hover hover:text-foreground focus-visible:bg-surface-hover focus-visible:text-foreground">
			<span>Previous: {prev.label}</span><kbd class={kbd}>P</kbd>
		</a>
	{/if}
	<a href={backHref} class="flex items-center justify-between gap-3 rounded-lg px-4 py-2 text-sm text-muted transition-colors hover:bg-surface-hover hover:text-foreground focus-visible:bg-surface-hover focus-visible:text-foreground">
		<span>Back to {item.title}</span><kbd class={kbd}>B</kbd>
	</a>
{/snippet}

<style>
	.reader-range {
		appearance: none;
		-webkit-appearance: none;
		height: 4px;
		flex: 1;
		cursor: pointer;
		border-radius: 999px;
		background: linear-gradient(
			to var(--to, right),
			var(--color-accent) var(--fill),
			var(--color-line) var(--fill)
		);
	}
	.reader-range::-webkit-slider-thumb {
		-webkit-appearance: none;
		width: 14px;
		height: 14px;
		border-radius: 999px;
		background: var(--color-foreground);
		border: 2px solid var(--color-accent);
		box-shadow: 0 2px 8px rgba(0, 0, 0, 0.5);
	}
	.reader-range::-moz-range-thumb {
		width: 10px;
		height: 10px;
		border-radius: 999px;
		background: var(--color-foreground);
		border: 2px solid var(--color-accent);
	}
</style>
