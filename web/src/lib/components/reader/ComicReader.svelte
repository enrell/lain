<script lang="ts">
	/*
	 * Comic adaptation of ReaderBase (D-085): left-to-right, one dense
	 * page at a time by default, fit and zoom for reading detail and
	 * lettering, an opt-in two-page spread for printed double pages, and
	 * a continuous scroll for digital-first titles.
	 */
	import Columns2 from '@lucide/svelte/icons/columns-2';
	import GalleryVertical from '@lucide/svelte/icons/gallery-vertical';
	import ZoomIn from '@lucide/svelte/icons/zoom-in';
	import ZoomOut from '@lucide/svelte/icons/zoom-out';
	import type { CatalogItem, ReaderView } from '$lib/api/types';
	import { normalizeSeriesTitle } from '$lib/utilities/grouping';
	import { readingLabel } from '$lib/reader/kinds';
	import { COMIC_PROFILE } from '$lib/reader/profile';
	import { ReaderSettings } from '$lib/reader/settings.svelte';
	import ReaderBase from './ReaderBase.svelte';
	import ReaderToggle from './ReaderToggle.svelte';

	let {
		item,
		view,
		startAt,
		backHref,
		prev = null,
		next = null,
		onPage
	}: {
		item: CatalogItem;
		view: ReaderView;
		startAt: number;
		backHref: string;
		prev?: { href: string; label: string } | null;
		next?: { href: string; label: string } | null;
		onPage?: (page: number, total: number) => void;
	} = $props();

	// svelte-ignore state_referenced_locally
	const settings = new ReaderSettings(COMIC_PROFILE, normalizeSeriesTitle(item.title), view.direction);
</script>

<ReaderBase
	{item}
	{view}
	{startAt}
	{settings}
	profile={COMIC_PROFILE}
	label={readingLabel(item)}
	{backHref}
	{prev}
	{next}
	{onPage}
>
	{#snippet toolbar()}
		{#if settings.mode === 'paged'}
			<ReaderToggle
				label={settings.fit === 'contain' ? 'Fit page to screen (click for full width)' : 'Full width (click to fit page)'}
				onclick={() => settings.setFit(settings.fit === 'contain' ? 'width' : 'contain')}
			>
				{settings.fit === 'contain' ? 'Fit' : 'Width'}
			</ReaderToggle>
			<ReaderToggle
				label="Zoom out (-)"
				disabled={settings.zoom <= 1}
				onclick={() => settings.setZoom(settings.zoom - 0.25)}
			>
				<ZoomOut class="size-4" aria-hidden="true" />
			</ReaderToggle>
			<span class="w-10 text-center font-mono text-[10px] tabular-nums text-muted">{Math.round(settings.zoom * 100)}%</span>
			<ReaderToggle
				label="Zoom in (+)"
				disabled={settings.zoom >= 3}
				onclick={() => settings.setZoom(settings.zoom + 0.25)}
			>
				<ZoomIn class="size-4" aria-hidden="true" />
			</ReaderToggle>
			<ReaderToggle
				label="Double-page spreads (D)"
				active={settings.dual}
				onclick={() => settings.toggleDual()}
			>
				<Columns2 class="size-4" aria-hidden="true" />
				<span class="hidden xl:inline">Spread</span>
			</ReaderToggle>
		{/if}
		<ReaderToggle
			label="Continuous scroll"
			active={settings.mode === 'strip'}
			onclick={() => settings.setMode(settings.mode === 'strip' ? 'paged' : 'strip')}
		>
			<GalleryVertical class="size-4" aria-hidden="true" />
			<span class="hidden xl:inline">Scroll</span>
		</ReaderToggle>
	{/snippet}
</ReaderBase>
