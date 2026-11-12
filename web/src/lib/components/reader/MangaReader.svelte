<script lang="ts">
	/*
	 * Manga adaptation of ReaderBase (D-085): right-to-left by default,
	 * two-page spreads on wide screens with the cover standing alone,
	 * a per-title direction override (a few titles print left to right)
	 * and a webtoon strip for long-scroll releases.
	 */
	import ArrowLeftRight from '@lucide/svelte/icons/arrow-left-right';
	import BookImage from '@lucide/svelte/icons/book-image';
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
				active={settings.direction === 'rtl'}
				onclick={() => settings.toggleDirection()}
			>
				<ArrowLeftRight class="size-4" aria-hidden="true" />
				{settings.direction}
			</ReaderToggle>
			<!-- Direction, then how pages are laid out, then the strip: three questions, kept apart.
			     Spreads exist only on a screen wider than tall, so an upright one does not offer them. -->
			<span class="flex items-center gap-1 portrait:hidden">
			<span class="mx-0.5 h-5 w-px shrink-0 bg-hairline" aria-hidden="true"></span>
			<ReaderToggle
				label="Two-page spreads (D)"
				active={settings.dual}
				onclick={() => settings.toggleDual()}
			>
				<Columns2 class="size-4" aria-hidden="true" />
				<span class="hidden max-md:inline xl:inline">Spread</span>
			</ReaderToggle>
			{#if settings.dual}
				<ReaderToggle
					label="Cover on its own page"
					active={settings.coverAlone}
					onclick={() => settings.toggleCoverAlone()}
				>
					<!-- An icon like its neighbours, the word where there is room for all the words. -->
					<BookImage class="size-4" aria-hidden="true" />
					<!-- An option of the spread, named as one: "Cover" beside "Spread" read as a third layout. -->
					<span class="hidden max-md:inline">Cover</span><span class="hidden xl:inline">Cover alone</span>
				</ReaderToggle>
			{/if}
			</span>
		{/if}
		<span class="mx-0.5 h-5 w-px shrink-0 bg-hairline" aria-hidden="true"></span>
		<ReaderToggle
			label="Webtoon strip"
			active={settings.mode === 'strip'}
			onclick={() => settings.setMode(settings.mode === 'strip' ? 'paged' : 'strip')}
		>
			<GalleryVertical class="size-4" aria-hidden="true" />
			<span class="hidden max-md:inline xl:inline">Webtoon</span>
		</ReaderToggle>
	{/snippet}
</ReaderBase>
