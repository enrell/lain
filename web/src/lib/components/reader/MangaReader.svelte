<script lang="ts">
	/*
	 * Manga adaptation of ReaderBase (D-085): right-to-left by default,
	 * two-page spreads on wide screens with the cover standing alone,
	 * a per-title direction override (a few titles print left to right)
	 * and a webtoon strip for long-scroll releases.
	 */
	import ArrowLeftRight from '@lucide/svelte/icons/arrow-left-right';
	import Columns2 from '@lucide/svelte/icons/columns-2';
	import GalleryVertical from '@lucide/svelte/icons/gallery-vertical';
	import type { CatalogItem, ReaderView } from '$lib/api/types';
	import { normalizeSeriesTitle } from '$lib/utilities/grouping';
	import { readingLabel } from '$lib/reader/kinds';
	import { MANGA_PROFILE } from '$lib/reader/profile';
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
	const settings = new ReaderSettings(MANGA_PROFILE, normalizeSeriesTitle(item.title), view.direction);
</script>

<ReaderBase
	{item}
	{view}
	{startAt}
	{settings}
	profile={MANGA_PROFILE}
	label={readingLabel(item)}
	{backHref}
	{prev}
	{next}
	{onPage}
>
	{#snippet toolbar()}
		{#if settings.mode === 'paged'}
			<ReaderToggle
				label={settings.direction === 'rtl' ? 'Reading right to left (R)' : 'Reading left to right (R)'}
				onclick={() => settings.toggleDirection()}
			>
				<ArrowLeftRight class="size-4" aria-hidden="true" />
				{settings.direction}
			</ReaderToggle>
			<ReaderToggle
				label="Two-page spreads (D)"
				active={settings.dual}
				onclick={() => settings.toggleDual()}
			>
				<Columns2 class="size-4" aria-hidden="true" />
				<span class="hidden xl:inline">Spread</span>
			</ReaderToggle>
			{#if settings.dual}
				<ReaderToggle
					label="Cover on its own page"
					active={settings.coverAlone}
					onclick={() => settings.toggleCoverAlone()}
				>
					Cover
				</ReaderToggle>
			{/if}
		{/if}
		<ReaderToggle
			label="Webtoon strip"
			active={settings.mode === 'strip'}
			onclick={() => settings.setMode(settings.mode === 'strip' ? 'paged' : 'strip')}
		>
			<GalleryVertical class="size-4" aria-hidden="true" />
			<span class="hidden xl:inline">Webtoon</span>
		</ReaderToggle>
	{/snippet}
</ReaderBase>
