<script lang="ts">
	/*
	 * The book reader (D-306): a reflowable book (EPUB, and MOBI/AZW3 the
	 * server converted) read chapter by chapter. Each sanitized chapter is
	 * shown in a frame that runs no script (sandboxed, same origin so this
	 * component can lay it out and listen to it). Paged, the chapter flows
	 * into viewport-wide columns and a page turn scrolls one viewport;
	 * scrolled, it is one long column. Progress is saved in the book's
	 * virtual pages (D-307), the same count every client shows.
	 */
	import { onDestroy, tick, untrack } from 'svelte';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import CaseSensitive from '@lucide/svelte/icons/case-sensitive';
	import CircleHelp from '@lucide/svelte/icons/circle-help';
	import ListTree from '@lucide/svelte/icons/list-tree';
	import Maximize from '@lucide/svelte/icons/maximize';
	import Minimize from '@lucide/svelte/icons/minimize';
	import { api } from '$lib/api';
	import type { BookManifest, BookTocEntry, CatalogItem } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Segmented from '$lib/components/primitives/Segmented.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import { t } from '$lib/i18n';
	import { leavePlayback } from '$lib/navigation/return';
	import {
		pagesIn,
		positionOfPage,
		progressAt,
		readerCss,
		withToken,
		type BookPosition,
		type BookProgress
	} from '$lib/reader/book';
	import { BookSettings, SIZE_MAX, SIZE_MIN } from '$lib/reader/book-settings.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let {
		item,
		book,
		start,
		backHref,
		onProgress
	}: {
		item: CatalogItem;
		book: BookManifest;
		/** Where to open. */
		start: BookPosition;
		backHref: string;
		/** Every move the reader makes; the route throttles the writes. */
		onProgress?: (p: BookProgress) => void;
	} = $props();

	const settings = new BookSettings();
	const chapters = $derived(book.chapters ?? []);
	const rtl = $derived(book.direction === 'rtl');

	let root = $state<HTMLDivElement>();
	let stage = $state<HTMLDivElement>();
	let frame = $state<HTMLIFrameElement>();
	let frameW = $state(0);
	let frameH = $state(0);

	// svelte-ignore state_referenced_locally
	let chapter = $state(start.chapter);
	let page = $state(0);
	let pageCount = $state(1);
	let fraction = $state(0);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let ui = $state(true);
	let toc = $state(false);
	let look = $state(false);
	let help = $state(false);
	let fullscreen = $state(false);
	let finished = $state(false);
	let jumping = $state(false);
	let jumpValue = $state('');
	let jumpInput = $state<HTMLInputElement>();

	/** Fetched chapters, by index (the next ones are fetched ahead). */
	const cache = new Map<number, Promise<string>>();
	function fetchChapter(n: number): Promise<string> {
		let p = cache.get(n);
		if (!p) {
			p = api.reader.chapter(item.id, n).then((html) => withToken(String(html ?? ''), item.id, session.token));
			p.catch(() => cache.delete(n));
			cache.set(n, p);
			if (cache.size > 6) cache.delete(cache.keys().next().value as number);
		}
		return p;
	}

	function appColors() {
		const s = root ? getComputedStyle(root) : null;
		const v = (n: string, d: string) => s?.getPropertyValue(n).trim() || d;
		return { background: v('--color-background', '#121212'), foreground: v('--color-foreground', '#e6e6e6'), accent: v('--color-accent', '#8ab4f8') };
	}

	const css = $derived(readerCss(settings.look(appColors()), frameW, frameH));

	function doc(): Document | null {
		return frame?.contentDocument ?? null;
	}

	function scroller(): HTMLElement | null {
		const d = doc();
		return (d?.scrollingElement as HTMLElement | null) ?? d?.documentElement ?? null;
	}

	/** Lays the current chapter out again and recounts its pages. */
	function measure(): void {
		const s = scroller();
		if (!s) return;
		if (settings.layout === 'paged') pageCount = pagesIn(s.scrollWidth, frameW);
		else pageCount = 1;
	}

	/** Shows page `p` of a paged chapter, or scrolls to `f` of a scrolled one. */
	function show(p: number): void {
		const s = scroller();
		if (!s) return;
		page = Math.min(Math.max(p, 0), pageCount - 1);
		s.scrollLeft = page * frameW;
		fraction = pageCount > 1 ? page / (pageCount - 1) : 0;
		report();
	}

	function report(): void {
		onProgress?.(progressAt({ chapter, fraction: fraction >= 0.999 && chapter === chapters.length - 1 ? 1 : fraction }, chapters));
	}

	type Target = { fraction?: number; last?: boolean; anchor?: string };

	let opening = 0;
	/** Opens chapter `n` at a place in it. */
	async function open(n: number, target: Target = {}): Promise<void> {
		if (n < 0 || n >= chapters.length) return;
		const ticket = ++opening;
		loading = true;
		error = null;
		finished = false;
		let html: string;
		try {
			html = await fetchChapter(n);
		} catch (err) {
			if (ticket !== opening) return;
			error = errorMessage(err);
			loading = false;
			return;
		}
		if (ticket !== opening || !frame) return;
		chapter = n;
		const styled = html.replace('</head>', `<style id="lain-reader">${css}</style></head>`);
		await new Promise<void>((resolve) => {
			const f = frame!;
			const done = () => {
				f.removeEventListener('load', done);
				resolve();
			};
			f.addEventListener('load', done);
			f.srcdoc = styled;
		});
		if (ticket !== opening) return;
		wire();
		await settle();
		land(target);
		loading = false;
		// Fetch the neighbours while this one is read.
		if (n + 1 < chapters.length) void fetchChapter(n + 1).catch(() => {});
		if (n > 0) void fetchChapter(n - 1).catch(() => {});
	}

	/** Waits for the chapter's pictures, which change its length. */
	async function settle(): Promise<void> {
		const d = doc();
		if (!d) return;
		const imgs = [...d.images].filter((i) => !i.complete);
		const loaded = (i: HTMLImageElement) =>
			new Promise<void>((r) => {
				i.addEventListener('load', () => r(), { once: true });
				i.addEventListener('error', () => r(), { once: true });
			});
		await Promise.race([Promise.all(imgs.map(loaded)), new Promise((r) => setTimeout(r, 1500))]);
		await tick();
		measure();
	}

	function land(target: Target): void {
		const d = doc();
		const s = scroller();
		if (!d || !s) return;
		if (target.anchor) {
			const el = d.getElementById(target.anchor) ?? d.getElementsByName(target.anchor)[0];
			if (el) {
				if (settings.layout === 'paged') {
					const x = el.getBoundingClientRect().left + s.scrollLeft;
					show(Math.floor(x / Math.max(frameW, 1)));
				} else {
					el.scrollIntoView({ block: 'start' });
					onScroll();
				}
				return;
			}
		}
		if (settings.layout === 'paged') {
			if (target.last) show(pageCount - 1);
			else show(Math.round((target.fraction ?? 0) * (pageCount - 1)));
		} else {
			const max = Math.max(s.scrollHeight - s.clientHeight, 0);
			s.scrollTop = target.last ? max : (target.fraction ?? 0) * max;
			onScroll();
		}
	}

	function onScroll(): void {
		if (settings.layout !== 'scroll') return;
		const s = scroller();
		if (!s) return;
		const max = Math.max(s.scrollHeight - s.clientHeight, 0);
		fraction = max > 0 ? Math.min(1, s.scrollTop / max) : 1;
		report();
	}

	/** Listens to the frame: keys, clicks on links and zones, scrolling, swipes. */
	function wire(): void {
		const d = doc();
		if (!d) return;
		d.addEventListener('keydown', onKey, true);
		d.addEventListener('click', onFrameClick);
		d.addEventListener('scroll', onScroll, { passive: true });
		let x0 = 0;
		let y0 = 0;
		d.addEventListener('touchstart', (e) => {
			x0 = e.touches[0]?.clientX ?? 0;
			y0 = e.touches[0]?.clientY ?? 0;
		}, { passive: true });
		d.addEventListener('touchend', (e) => {
			if (settings.layout !== 'paged') return;
			const dx = (e.changedTouches[0]?.clientX ?? 0) - x0;
			const dy = (e.changedTouches[0]?.clientY ?? 0) - y0;
			if (Math.abs(dx) > 50 && Math.abs(dx) > Math.abs(dy) * 1.5) turn(dx < 0 === !rtl ? 1 : -1);
		}, { passive: true });
	}

	function onFrameClick(e: MouseEvent): void {
		const a = (e.target as Element | null)?.closest?.('a');
		if (a) {
			const ch = a.getAttribute('data-lain-chapter');
			const href = a.getAttribute('href') ?? '';
			if (ch !== null) {
				e.preventDefault();
				const anchor = a.getAttribute('data-lain-anchor') ?? '';
				const n = Number(ch);
				if (n === chapter && anchor) land({ anchor });
				else void open(n, { anchor });
				return;
			}
			if (href.startsWith('#')) {
				e.preventDefault();
				land({ anchor: decodeURIComponent(href.slice(1)) });
				return;
			}
			// An outside link opens in a new tab (target=_blank from the server).
			return;
		}
		// Selecting text is not a page turn.
		if (doc()?.getSelection()?.toString()) return;
		const w = frameW || 1;
		const x = e.clientX;
		if (settings.layout === 'scroll' || (x > w * 0.3 && x < w * 0.7)) {
			ui = !ui;
			return;
		}
		turn((x >= w * 0.7) !== rtl ? 1 : -1);
	}

	const TURN_GAP_MS = 110;
	let lastTurn = 0;
	/** One page forward (+1) or back (-1) in reading order, across chapters. */
	function turn(dir: 1 | -1): void {
		if (loading) return;
		const now = performance.now();
		if (now - lastTurn < TURN_GAP_MS) return;
		lastTurn = now;
		ui = false;
		if (settings.layout === 'scroll') {
			const s = scroller();
			if (!s) return;
			const max = s.scrollHeight - s.clientHeight;
			if (dir > 0 && s.scrollTop >= max - 2) return void nextChapter();
			if (dir < 0 && s.scrollTop <= 2) return void prevChapter(true);
			s.scrollBy({ top: dir * s.clientHeight * 0.9 });
			return;
		}
		if (dir > 0 && page >= pageCount - 1) return void nextChapter();
		if (dir < 0 && page <= 0) return void prevChapter(true);
		show(page + dir);
	}

	function nextChapter(): void {
		if (chapter + 1 >= chapters.length) {
			finished = true;
			ui = true;
			fraction = 1;
			report();
			return;
		}
		void open(chapter + 1, { fraction: 0 });
	}

	function prevChapter(atEnd = false): void {
		if (chapter <= 0) return;
		void open(chapter - 1, atEnd ? { last: true } : { fraction: 0 });
	}

	// A new look or a new size: the stylesheet is swapped in place and the
	// reader stays where it was.
	$effect(() => {
		const sheet = css;
		const layout = settings.layout;
		untrack(() => {
			const d = doc();
			const el = d?.getElementById('lain-reader');
			if (!d || !el || loading) return;
			const keep = fraction;
			el.textContent = sheet;
			void tick().then(() => {
				measure();
				if (layout === 'paged') show(Math.round(keep * (pageCount - 1)));
				else land({ fraction: keep });
			});
		});
	});

	// The first chapter, once the frame has a size.
	let started = false;
	$effect(() => {
		if (started || !frame || frameW === 0 || frameH === 0) return;
		started = true;
		untrack(() => void open(start.chapter, { fraction: start.fraction }));
	});

	// Global position: the virtual page of 1024 characters (D-307).
	const progress = $derived(progressAt({ chapter, fraction }, chapters));
	const current = $derived(Math.max(1, Math.floor(progress.position_sec)));
	const percent = $derived(Math.round((progress.position_sec / Math.max(book.pages, 1)) * 100));

	function seek(e: Event): void {
		const v = Number((e.currentTarget as HTMLInputElement).value);
		const pos = positionOfPage(v, chapters);
		if (pos.chapter === chapter) land({ fraction: pos.fraction });
		else void open(pos.chapter, { fraction: pos.fraction });
	}

	function openJump(): void {
		jumpValue = String(current);
		jumping = true;
		void tick().then(() => jumpInput?.select());
	}

	function submitJump(): void {
		jumping = false;
		const n = Number.parseInt(jumpValue, 10);
		if (!Number.isFinite(n)) return;
		const pos = positionOfPage(Math.min(Math.max(n, 1), book.pages), chapters);
		if (pos.chapter === chapter) land({ fraction: pos.fraction });
		else void open(pos.chapter, { fraction: pos.fraction });
	}

	function goToc(e: BookTocEntry): void {
		toc = false;
		if (e.chapter < 0) return;
		if (e.chapter === chapter && e.anchor) land({ anchor: e.anchor });
		else void open(e.chapter, e.anchor ? { anchor: e.anchor } : { fraction: 0 });
	}

	// The entry being read: the last one at or before this chapter.
	const flatToc = $derived.by(() => {
		const out: { entry: BookTocEntry; depth: number }[] = [];
		const walk = (list: BookTocEntry[], depth: number) => {
			for (const e of list) {
				out.push({ entry: e, depth });
				if (e.children) walk(e.children, depth + 1);
			}
		};
		walk(book.toc ?? [], 0);
		return out;
	});
	const here = $derived(
		flatToc.reduce<BookTocEntry | null>((best, { entry }) => (entry.chapter >= 0 && entry.chapter <= chapter ? entry : best), null)
	);

	async function toggleFullscreen(): Promise<void> {
		try {
			if (document.fullscreenElement) await document.exitFullscreen();
			else await root?.requestFullscreen();
		} catch {
			/* Fullscreen is a convenience; a refusal changes nothing. */
		}
	}

	const shortcuts = $derived<[string[], string][]>([
		[[rtl ? '←' : '→', 'Space'], t('reader.keys.next')],
		[[rtl ? '→' : '←', 'Shift+Space'], t('reader.keys.previous')],
		[['Home', 'End'], t('book.keys.ends')],
		[['N', ']'], t('book.keys.nextChapter')],
		[['P', '['], t('book.keys.previousChapter')],
		[['T'], t('book.keys.toc')],
		[['A'], t('book.keys.look')],
		[['+', '-', '0'], t('book.keys.size')],
		[['M'], t('book.keys.layout')],
		[['G'], t('reader.keys.jump')],
		[['B', 'Esc'], t('reader.keys.back')],
		[['H'], t('reader.keys.bars')],
		[['F'], t('reader.keys.fullscreen')],
		[['?'], t('shortcuts.help')]
	]);

	// The Escape that closes a dialog must not also leave the reader.
	let closedAt = 0;
	$effect(() => {
		if (!help && !toc && !look) closedAt = performance.now();
	});

	function onKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (e.key === 'Escape' && performance.now() - closedAt < 250) return;
		if (help || toc || look) {
			if (e.key === '?' && help) {
				e.preventDefault();
				help = false;
			}
			return;
		}
		if (jumping) return;
		const target = e.target as HTMLElement | null;
		const tag = target?.tagName;
		if (tag === 'TEXTAREA' || tag === 'SELECT' || (tag === 'INPUT' && (target as HTMLInputElement).type !== 'range')) return;
		if ((e.key === ' ' || e.key === 'Enter') && (tag === 'BUTTON' || tag === 'A') && target?.ownerDocument === document) return;
		const k = e.key.toLowerCase();
		const handled = (): void => e.preventDefault();
		if (finished) {
			if (e.key === 'Escape' || k === 'b' || e.key === 'Backspace') {
				handled();
				return leavePlayback(backHref);
			}
			if (e.key === (rtl ? 'ArrowRight' : 'ArrowLeft')) {
				handled();
				finished = false;
			}
			return;
		}
		switch (true) {
			case e.key === '?':
				handled();
				help = true;
				return;
			case e.key === 'ArrowRight' || e.key === 'PageDown' || (e.key === ' ' && !e.shiftKey):
				handled();
				return turn(e.key === 'ArrowRight' && rtl ? -1 : 1);
			case e.key === 'ArrowLeft' || e.key === 'PageUp' || (e.key === ' ' && e.shiftKey):
				handled();
				return turn(e.key === 'ArrowLeft' && rtl ? 1 : -1);
			case (e.key === 'ArrowDown' || e.key === 'ArrowUp') && settings.layout === 'scroll':
				handled();
				scroller()?.scrollBy({ top: (e.key === 'ArrowDown' ? 1 : -1) * 80 });
				return;
			case e.key === 'Home':
				handled();
				return land({ fraction: 0 });
			case e.key === 'End':
				handled();
				return land({ last: true });
			case k === 'n' || e.key === ']':
				handled();
				return nextChapter();
			case k === 'p' || e.key === '[':
				handled();
				return prevChapter();
			case k === 't':
				handled();
				toc = true;
				return;
			case k === 'a':
				handled();
				look = true;
				return;
			case e.key === '+' || e.key === '=':
				handled();
				return settings.bigger();
			case e.key === '-':
				handled();
				return settings.smaller();
			case e.key === '0':
				handled();
				return settings.setSize(100);
			case k === 'm':
				handled();
				return settings.toggleLayout();
			case k === 'g':
				handled();
				return openJump();
			case k === 'b' || e.key === 'Backspace' || e.key === 'Escape':
				handled();
				return leavePlayback(backHref);
			case k === 'h':
				handled();
				ui = !ui;
				return;
			case k === 'f':
				handled();
				return void toggleFullscreen();
		}
	}

	// The frame is as big as the stage; a resize lays the chapter out again.
	$effect(() => {
		if (!stage) return;
		const ro = new ResizeObserver(() => {
			frameW = stage?.clientWidth ?? 0;
			frameH = stage?.clientHeight ?? 0;
		});
		ro.observe(stage);
		return () => ro.disconnect();
	});

	onDestroy(() => {
		opening++;
	});

	const fonts = $derived(BookSettings.fonts.map((f) => ({ value: f, label: t(`book.look.font.${f}`) })));
	const themes = $derived(BookSettings.themes.map((v) => ({ value: v, label: t(`book.look.theme.${v}`) })));
	const layouts = $derived([
		{ value: 'paged' as const, label: t('book.look.paged') },
		{ value: 'scroll' as const, label: t('book.look.scroll') }
	]);
	const spacings = $derived(BookSettings.lineHeights.map((v) => ({ value: String(v), label: String(v) })));
	const margins = $derived(
		BookSettings.margins.map((v, i) => ({ value: String(v), label: t(['book.look.narrow', 'book.look.normal', 'book.look.wide'][i] as 'book.look.narrow') }))
	);
</script>

<svelte:window onkeydown={onKey} />
<svelte:document onfullscreenchange={() => (fullscreen = !!document.fullscreenElement)} />

<div bind:this={root} class="fixed inset-0 z-40 flex flex-col bg-background text-foreground" data-immersive data-book-reader data-loading={loading ? "true" : "false"}>
	<header
		class="pointer-events-none absolute inset-x-0 top-0 z-20 p-3 transition duration-300 ease-[var(--ease-out-quick)] {ui ? '' : '-translate-y-[130%] opacity-0'}"
	>
		<div class="pointer-events-auto mx-auto flex max-w-5xl items-center gap-2 rounded-2xl border border-line/70 bg-background/85 p-2 shadow-[0_18px_45px_rgba(0,0,0,0.35)] backdrop-blur-xl">
			<a
				href={backHref}
				onclick={(e) => {
					e.preventDefault();
					leavePlayback(backHref);
				}}
				class="inline-flex size-11 shrink-0 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground sm:size-9"
				aria-label={t('reader.back', { title: item.title })}
			>
				<ArrowLeft class="size-[18px]" aria-hidden="true" />
			</a>
			<div class="min-w-0 flex-1 px-1">
				<p class="truncate font-mono text-[10px] font-semibold uppercase tracking-[0.22em] text-accent">
					{here?.title || t('book.chapter', { number: chapter + 1 })}
				</p>
				<h1 class="truncate text-sm font-semibold tracking-[-0.01em] text-foreground">{book.title || item.title}</h1>
			</div>
			<p class="shrink-0 px-2 font-mono text-xs tabular-nums text-muted max-sm:hidden" aria-live="polite">
				{t('book.pageOf', { page: current, pages: book.pages })}
			</p>
			<div class="flex shrink-0 items-center gap-0.5">
				<button
					type="button"
					class="inline-flex size-11 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground sm:size-9"
					aria-label={t('book.toc')}
					disabled={!book.toc?.length}
					onclick={() => (toc = true)}
				>
					<ListTree class="size-4" aria-hidden="true" />
				</button>
				<button
					type="button"
					class="inline-flex size-11 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground sm:size-9"
					aria-label={t('book.look.title')}
					onclick={() => (look = true)}
				>
					<CaseSensitive class="size-[18px]" aria-hidden="true" />
				</button>
				<button
					type="button"
					class="inline-flex size-11 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground sm:size-9"
					aria-label={fullscreen ? t('reader.exitFullscreen') : t('reader.fullscreen')}
					onclick={toggleFullscreen}
				>
					{#if fullscreen}<Minimize class="size-4" aria-hidden="true" />{:else}<Maximize class="size-4" aria-hidden="true" />{/if}
				</button>
				<button
					type="button"
					class="inline-flex size-9 items-center justify-center rounded-lg text-muted transition-colors hover:bg-surface-hover hover:text-foreground max-sm:hidden"
					aria-label={t('reader.shortcuts')}
					aria-pressed={help}
					onclick={() => (help = !help)}
				>
					<CircleHelp class="size-4" aria-hidden="true" />
				</button>
			</div>
		</div>
	</header>

	<div bind:this={stage} class="relative min-h-0 flex-1" data-book-stage>
		<iframe
			bind:this={frame}
			title={t('book.frameTitle', { title: book.title || item.title })}
			sandbox="allow-same-origin allow-popups allow-popups-to-escape-sandbox"
			class="absolute inset-0 block border-0 {loading ? 'opacity-0' : 'opacity-100'} transition-opacity duration-150"
			style:width="{frameW}px"
			style:height="{frameH}px"
		></iframe>
		{#if loading && !error}
			<div class="absolute inset-0 flex items-center justify-center">
				<Spinner class="size-6 text-accent" label={t('book.opening')} />
			</div>
		{/if}
		{#if error}
			<div class="absolute inset-0 flex items-center justify-center p-8">
				<ErrorState message={error} retry={() => void open(chapter, { fraction })} />
			</div>
		{/if}
	</div>

	{#if !ui}
		<p class="pointer-events-none absolute bottom-2 left-1/2 z-10 -translate-x-1/2 font-mono text-[10px] tabular-nums tracking-[0.14em] text-muted/80">
			{t('book.pageOf', { page: current, pages: book.pages })}
		</p>
	{/if}

	{#if finished}
		<div class="absolute inset-0 z-30 flex items-center justify-center bg-black/60 p-6 backdrop-blur-sm">
			<div role="dialog" aria-modal="true" aria-label={t('book.end.kicker')} class="w-full max-w-sm rounded-2xl border border-line bg-surface p-6 text-center shadow-2xl">
				<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent">{t('book.end.kicker')}</p>
				<p class="mt-2 text-xl font-semibold tracking-[-0.02em] text-foreground">{book.title || item.title}</p>
				{#if book.authors?.length}<p class="mt-1 text-sm text-muted">{book.authors.join(', ')}</p>{/if}
				<div class="mt-6 flex flex-col gap-2">
					<a
						href={backHref}
						data-primary
						onclick={(e) => {
							e.preventDefault();
							leavePlayback(backHref);
						}}
						class="flex items-center justify-between gap-3 rounded-lg bg-accent px-4 py-2.5 text-sm font-semibold text-accent-fg hover:bg-accent-hover"
					>
						<span>{t('reader.end.back', { title: item.title })}</span><kbd>B</kbd>
					</a>
					<button
						type="button"
						class="flex items-center justify-between gap-3 rounded-lg px-4 py-2 text-sm text-muted hover:bg-surface-hover hover:text-foreground"
						onclick={() => (finished = false)}
					>
						<span>{t('book.end.stay')}</span><kbd>{rtl ? '→' : '←'}</kbd>
					</button>
				</div>
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
				<label class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-accent" for="book-jump">{t('reader.jump.label')}</label>
				<div class="mt-3 flex items-center gap-2">
					<input
						id="book-jump"
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
					<span class="shrink-0 font-mono text-xs tabular-nums text-muted">/ {book.pages}</span>
				</div>
				<p class="mt-2 font-mono text-[10px] text-muted"><kbd>Enter</kbd> {t('reader.jump.go')} · <kbd>Esc</kbd> {t('reader.jump.cancel')}</p>
			</form>
		</div>
	{/if}

	<footer
		class="pointer-events-none absolute inset-x-0 bottom-0 z-20 p-3 transition duration-300 ease-[var(--ease-out-quick)] {ui ? '' : 'translate-y-[130%] opacity-0'}"
	>
		<div class="pointer-events-auto mx-auto flex max-w-3xl items-center gap-3 rounded-2xl border border-line/70 bg-background/85 px-4 py-3 shadow-[0_18px_45px_rgba(0,0,0,0.35)] backdrop-blur-xl">
			<div class="flex min-w-0 flex-1 items-center gap-3" dir={rtl ? 'rtl' : 'ltr'}>
				<span class="font-mono text-[10px] tabular-nums text-muted">1</span>
				<input
					type="range"
					min="1"
					max={Math.max(book.pages, 1)}
					value={current}
					onchange={seek}
					onpointerup={(e) => e.currentTarget.blur()}
					tabindex="-1"
					aria-label={t('reader.page')}
					class="book-range"
					style:--fill="{book.pages > 1 ? ((current - 1) / (book.pages - 1)) * 100 : 100}%"
					style:--to={rtl ? 'left' : 'right'}
				/>
				<span class="font-mono text-[10px] tabular-nums text-muted">{book.pages}</span>
			</div>
			<span class="shrink-0 font-mono text-[10px] tabular-nums text-muted">{t('book.percent', { percent })}</span>
		</div>
	</footer>
</div>

<Modal bind:open={toc} title={t('book.toc')}>
	<nav aria-label={t('book.toc')} class="-mx-2 max-h-[60vh] overflow-y-auto">
		<ul class="flex flex-col">
			{#each flatToc as { entry, depth }, i (i)}
				<li>
					<button
						type="button"
						data-autofocus={entry === here ? true : undefined}
						disabled={entry.chapter < 0}
						class="flex w-full items-baseline gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors hover:bg-surface-hover focus-visible:bg-surface-hover disabled:opacity-50 {entry === here ? 'text-accent' : 'text-foreground'}"
						style:padding-left="{0.5 + depth * 1}rem"
						onclick={() => goToc(entry)}
					>
						<span class="min-w-0 flex-1 truncate">{entry.title}</span>
					</button>
				</li>
			{/each}
		</ul>
	</nav>
</Modal>

<Modal bind:open={look} title={t('book.look.title')}>
	<div class="flex flex-col gap-4 text-sm">
		<div class="flex items-center justify-between gap-3">
			<span class="text-muted">{t('book.look.size')}</span>
			<div class="flex items-center gap-2">
				<button type="button" class="inline-flex size-9 items-center justify-center rounded-lg border border-line hover:bg-surface-hover disabled:opacity-40" aria-label={t('book.look.smaller')} disabled={settings.size <= SIZE_MIN} onclick={() => settings.smaller()}>A−</button>
				<span class="w-12 text-center font-mono text-xs tabular-nums">{settings.size}%</span>
				<button type="button" class="inline-flex size-9 items-center justify-center rounded-lg border border-line hover:bg-surface-hover disabled:opacity-40" aria-label={t('book.look.bigger')} disabled={settings.size >= SIZE_MAX} onclick={() => settings.bigger()}>A+</button>
			</div>
		</div>
		<div class="flex flex-col gap-1.5">
			<span class="text-muted">{t('book.look.fontLabel')}</span>
			<Segmented size="sm" label={t('book.look.fontLabel')} options={fonts} value={settings.font} onchange={(v) => settings.setFont(v)} />
		</div>
		<div class="flex flex-col gap-1.5">
			<span class="text-muted">{t('book.look.themeLabel')}</span>
			<Segmented size="sm" label={t('book.look.themeLabel')} options={themes} value={settings.theme} onchange={(v) => settings.setTheme(v)} />
		</div>
		<div class="flex flex-col gap-1.5">
			<span class="text-muted">{t('book.look.layoutLabel')} <kbd>M</kbd></span>
			<Segmented size="sm" label={t('book.look.layoutLabel')} options={layouts} value={settings.layout} onchange={(v) => settings.setLayout(v)} />
		</div>
		<div class="flex flex-col gap-1.5">
			<span class="text-muted">{t('book.look.spacing')}</span>
			<Segmented size="sm" label={t('book.look.spacing')} options={spacings} value={String(settings.lineHeight)} onchange={(v) => settings.setLineHeight(Number(v))} />
		</div>
		<div class="flex flex-col gap-1.5">
			<span class="text-muted">{t('book.look.margins')}</span>
			<Segmented size="sm" label={t('book.look.margins')} options={margins} value={String(settings.margin)} onchange={(v) => settings.setMargin(Number(v))} />
		</div>
		<label class="flex items-center justify-between gap-3">
			<span class="text-muted">{t('book.look.justify')}</span>
			<input type="checkbox" class="size-4 accent-[var(--color-accent)]" checked={settings.justify} onchange={(e) => settings.setJustify(e.currentTarget.checked)} />
		</label>
	</div>
</Modal>

<Modal bind:open={help} title={t('shortcuts.title')}>
	<dl class="mt-2 grid grid-cols-[auto_1fr] items-center gap-x-5 gap-y-2.5 text-sm">
		{#each shortcuts as [keys, what] (what)}
			<dt class="flex gap-1">{#each keys as key (key)}<kbd>{key}</kbd>{/each}</dt>
			<dd class="text-muted">{what}</dd>
		{/each}
	</dl>
</Modal>

<style>
	.book-range {
		appearance: none;
		-webkit-appearance: none;
		height: 4px;
		flex: 1;
		cursor: pointer;
		border-radius: 999px;
		background: linear-gradient(to var(--to, right), var(--color-accent) var(--fill), var(--color-line) var(--fill));
	}
	.book-range::-webkit-slider-thumb {
		-webkit-appearance: none;
		width: 14px;
		height: 14px;
		border-radius: 999px;
		background: var(--color-foreground);
		border: 2px solid var(--color-accent);
	}
	.book-range::-moz-range-thumb {
		width: 10px;
		height: 10px;
		border-radius: 999px;
		background: var(--color-foreground);
		border: 2px solid var(--color-accent);
	}
</style>
