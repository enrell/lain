<script lang="ts">
	/*
	 * Manga adaptation of ReaderBase (D-085): right-to-left by default,
	 * two-page spreads on wide screens with the cover standing alone,
	 * a per-title direction override (a few titles print left to right)
	 * and a webtoon strip for long-scroll releases.
	 */
	import BookImage from '@lucide/svelte/icons/book-image';
	import Columns2 from '@lucide/svelte/icons/columns-2';
	import GalleryVertical from '@lucide/svelte/icons/gallery-vertical';
	import type { CatalogItem, ReaderView } from '$lib/api/types';
	import { normalizeSeriesTitle } from '$lib/utilities/grouping';
	import { readingLabel } from '$lib/reader/kinds';
	import { MANGA_PROFILE } from '$lib/reader/profile';
	import { ReaderSettings } from '$lib/reader/settings.svelte';
	import ReaderBase from './ReaderBase.svelte';
	import ReaderGroup from './ReaderGroup.svelte';
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
			<!-- Direction, then how pages are laid out, then the strip: three questions, each in
			     its own track. Both directions are named, so left to right is a lit "LTR", not
			     only an unlit "RTL". -->
			<ReaderGroup label="Reading direction">
				<ReaderToggle
					label="Read left to right (R)"
					active={settings.direction === 'ltr'}
					onclick={() => settings.setDirection('ltr')}
				>
					LTR
				</ReaderToggle>
				<ReaderToggle
					label="Read right to left (R)"
					active={settings.direction === 'rtl'}
					onclick={() => settings.setDirection('rtl')}
				>
					RTL
				</ReaderToggle>
			</ReaderGroup>
			<!-- Spreads exist only on a screen wider than tall, so an upright one does not offer them. -->
			<ReaderGroup label="Page layout" class="portrait:hidden">
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
			</ReaderGroup>
		{/if}
		<ReaderGroup label="Strip">
			<ReaderToggle
				label="Webtoon strip"
				active={settings.mode === 'strip'}
				onclick={() => settings.setMode(settings.mode === 'strip' ? 'paged' : 'strip')}
			>
				<GalleryVertical class="size-4" aria-hidden="true" />
				<span class="hidden max-md:inline xl:inline">Webtoon</span>
			</ReaderToggle>
		</ReaderGroup>
	{/snippet}
</ReaderBase>
