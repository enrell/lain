<script lang="ts">
	import { onDestroy } from 'svelte';
	import { page } from '$app/state';
	import type { CatalogItem, Progress, ReaderView } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import ComicReader from '$lib/components/reader/ComicReader.svelte';
	import MangaReader from '$lib/components/reader/MangaReader.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { itemCache } from '$lib/stores/media-cache.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { compareEpisodes } from '$lib/utilities/grouping';
	import { ProgressReporter } from '$lib/utilities/progress-reporter';
	import { isReadable, readingLabel } from '$lib/reader/kinds';
	import { progressFor, startPage } from '$lib/reader/layout';

	const id = $derived(page.params.id ?? '');

	let item = $state<CatalogItem | null>(null);
	let view = $state<ReaderView | null>(null);
	let progress = $state<Progress | null>(null);
	let siblings = $state<CatalogItem[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);

	// Page turns are frequent and cheap to lose one of, so writes are
	// throttled and coalesced like playback progress.
	let reporter: ProgressReporter | null = null;
	let opened = $state(-1);

	async function load(): Promise<void> {
		await reporter?.flush();
		loading = true;
		error = null;
		notFound = false;
		item = null;
		view = null;
		try {
			const it = await api.catalog.get(id);
			if (!isReadable(it.kind)) {
				error = 'This item is not a comic or manga.';
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
			const target = id;
			reporter = new ProgressReporter((s) => api.progress.put(target, s), {
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
		if (pageIndex === opened) return;
		const snap = progressFor(pageIndex, total);
		void reporter?.update(snap, { force: snap.completed || pageIndex === 0 });
	}

	onDestroy(() => void reporter?.flush());

	const ordered = $derived([...siblings].filter((s) => !s.missing).sort(compareEpisodes));
	const at = $derived(ordered.findIndex((s) => s.id === id));
	function neighbour(target: CatalogItem | undefined) {
		return target ? { href: `/read/${target.id}`, label: readingLabel(target) } : null;
	}
	const prev = $derived(at > 0 ? neighbour(ordered[at - 1]) : null);
	const next = $derived(at >= 0 ? neighbour(ordered[at + 1]) : null);
</script>

<svelte:head><title>{item?.title ? `Reading ${item.title} — Lain` : 'Reader — Lain'}</title></svelte:head>

{#if loading && !item}
	<div class="flex h-dvh items-center justify-center bg-background">
		<Spinner class="size-6 text-accent" label="Opening" />
	</div>
{:else if notFound}
	<div class="p-8"><ErrorState message="This item is not in your library." /></div>
{:else if error || !item || !view}
	<div class="p-8"><ErrorState message={error ?? 'Unknown error'} retry={() => void load()} /></div>
{:else}
	{#key item.id}
		{#if item.kind === 'manga'}
			<MangaReader {item} {view} startAt={opened} backHref={`/item/${item.id}`} {prev} {next} {onPage} />
		{:else}
			<ComicReader {item} {view} startAt={opened} backHref={`/item/${item.id}`} {prev} {next} {onPage} />
		{/if}
	{/key}
{/if}
