<script lang="ts">
	import { onDestroy } from 'svelte';
	import { page } from '$app/state';
	import type { BookManifest, CatalogItem, Progress, ReaderView } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import ComicReader from '$lib/components/reader/ComicReader.svelte';
	import MangaReader from '$lib/components/reader/MangaReader.svelte';
	import BookReader from '$lib/components/reader/BookReader.svelte';
	import PdfReader from '$lib/components/reader/PdfReader.svelte';
	import { positionOf, type BookPosition, type BookProgress } from '$lib/reader/book';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { itemCache } from '$lib/stores/media-cache.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { compareEpisodes } from '$lib/utilities/grouping';
	import { ProgressReporter } from '$lib/utilities/progress-reporter';
	import { isReadable, readingLabel } from '$lib/reader/kinds';
	import { startPage } from '$lib/reader/layout';
	import { PageTurns } from '$lib/reader/turns';
	import { t } from '$lib/i18n';

	const id = $derived(page.params.id ?? '');

	let item = $state<CatalogItem | null>(null);
	let view = $state<ReaderView | null>(null);
	let book = $state<BookManifest | null>(null);
	let bookStart = $state<BookPosition>({ chapter: 0, fraction: 0 });
	let progress = $state<Progress | null>(null);
	let siblings = $state<CatalogItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);

	// Page turns are frequent and cheap to lose one of, so writes are
	// throttled and coalesced like playback progress.
	let reporter: ProgressReporter | null = null;
	let opened = $state(-1);
	// Which turn is still owed to the server, across files (see saveOnHide).
	const turns = new PageTurns();

	async function load(): Promise<void> {
		await reporter?.flush();
		loading = true;
		error = null;
		notFound = false;
		item = null;
		view = null;
		book = null;
		try {
			const it = await api.catalog.get(id);
			if (!isReadable(it.kind)) {
				error = t('reader.notReadable');
				return;
			}
			if (it.kind === 'book') {
				const [m, prog] = await Promise.all([api.reader.book(id), api.progress.get(id).catch(() => null)]);
				item = it;
				itemCache.set(it.id, it);
				book = m;
				progress = prog;
				siblings = [];
				const target = id;
				reporter = new ProgressReporter((s) => api.progress.put(target, s).then(() => turns.saved(target, s)), {
					intervalMs: 2000,
					minDeltaSec: 0.05
				});
				if (m.layout === 'pages') {
					view = await api.reader.pages(id);
					opened = startPage(prog, view.pages.length);
					turns.open(opened);
				} else {
					bookStart = positionOf(prog, m.chapters ?? []);
				}
				return;
			}
			const [v, prog, eps] = await Promise.all([
				api.reader.pages(id),
				api.progress.get(id).catch(() => null),
				api.catalog.episodes(id)
			]);
			item = it;
			itemCache.set(it.id, it);
			view = v;
			progress = prog;
			siblings = eps.items;
			opened = startPage(prog, v.pages.length);
			turns.open(opened);
			const target = id;
			// A landed write settles the turn it carried, so hiding the tab
			// does not send it a second time.
			reporter = new ProgressReporter((s) => api.progress.put(target, s).then(() => turns.saved(target, s)), {
				intervalMs: 2000,
				minDeltaSec: 1
			});
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'not-found') notFound = true;
			else error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	// SvelteKit reuses this component when only [id] changes (next volume).
	$effect(() => {
		void id;
		void load();
	});

	function onPage(pageIndex: number, total: number): void {
		// Opening a finished volume must not reset it: only a real page turn counts.
		const snap = turns.turn(id, pageIndex, total);
		if (!snap) return;
		void reporter?.update(snap, { force: snap.completed || pageIndex === 0 });
	}

	// A reflowable book reports its virtual page (D-307); the newest one is
	// owed to the server until it lands, like a page turn.
	let bookMoved = false;
	function onBookProgress(p: BookProgress): void {
		if (!bookMoved && progress && Math.abs(p.position_sec - progress.position_sec) < 0.01) return;
		bookMoved = true;
		turns.owe(id, p);
		void reporter?.update(p, { force: p.completed });
	}

	onDestroy(() => void reporter?.flush());

	/*
	 * Closing the tab or reloading never runs onDestroy, and the turns made
	 * within the reporter's interval were lost with it: page 4 read, page 2
	 * kept. The newest page goes out with keepalive as the page hides, the
	 * way the player saves its position.
	 */
	function saveOnHide(): void {
		const owed = turns.take();
		if (!owed) return;
		const { id: target, snap } = owed;
		void api.progress.put(target, snap, { keepalive: true }).catch(() => {
			// The page is going away; there is no UI left to inform.
		});
	}

	const ordered = $derived([...siblings].filter((s) => !s.missing).sort(compareEpisodes));
	const at = $derived(ordered.findIndex((s) => s.id === id));
	function neighbour(target: CatalogItem | undefined) {
		return target ? { href: `/read/${target.id}`, label: readingLabel(target) } : null;
	}
	const prev = $derived(at > 0 ? neighbour(ordered[at - 1]) : null);
	const next = $derived(at >= 0 ? neighbour(ordered[at + 1]) : null);
</script>

<svelte:head><title>{item?.title ? t('reader.pageTitle', { title: item.title }) : t('reader.pageTitleEmpty')}</title></svelte:head>
<svelte:window onpagehide={saveOnHide} />
<svelte:document onvisibilitychange={() => document.visibilityState === 'hidden' && saveOnHide()} />

{#if loading && !item}
	<div class="flex h-dvh items-center justify-center bg-background">
		<Spinner class="size-6 text-accent" label={t('reader.opening')} />
	</div>
{:else if notFound}
	<div class="p-8"><ErrorState message={t('reader.notFound')} /></div>
{:else if item && book && book.layout !== 'pages'}
	{#key item.id}
		<BookReader {item} {book} start={bookStart} backHref={`/item/${item.id}`} onProgress={onBookProgress} />
	{/key}
{:else if item && book && view}
	{#key item.id}
		<PdfReader {item} {view} title={book.title} startAt={opened} backHref={`/item/${item.id}`} {onPage} />
	{/key}
{:else if error || !item || !view}
	<div class="p-8"><ErrorState message={error ?? t('reader.unknownError')} retry={() => void load()} /></div>
{:else}
	{#key item.id}
		{#if item.kind === 'manga'}
			<MangaReader {item} {view} startAt={opened} backHref={`/item/${item.id}`} {prev} {next} {onPage} />
		{:else}
			<ComicReader {item} {view} startAt={opened} backHref={`/item/${item.id}`} {prev} {next} {onPage} />
		{/if}
	{/key}
{/if}
